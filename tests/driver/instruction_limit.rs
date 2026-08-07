//! `limits.lua_instruction_limit` was parsed and never read: a `while true do
//! end` in any command wedged the single game thread permanently.
//!
//! Like `sandbox_reality_check`, every probe here runs in the VM the engine
//! builds. Testing the budget arithmetic in isolation would pass just as
//! happily with nothing installing the hook — and in this case it would also
//! miss the thing that actually decides whether the limit works: LuaJIT
//! dispatches no hooks from inside a compiled trace, so the engine has to turn
//! the compiler off before a budget means anything. Only a test that boots the
//! real engine sees that.

use std::time::{Duration, Instant};

use crate::common::RealVm;

/// The whole point: an unbounded loop has to come back, and quickly.
#[test]
fn a_runaway_loop_is_stopped_instead_of_wedging_the_thread() {
    let mut vm = RealVm::boot_with_instruction_limit(200_000);

    let started = Instant::now();
    let err = vm.eval("while true do end").err();
    let elapsed = started.elapsed();

    assert!(
        err.contains("instruction limit exceeded"),
        "expected the budget to raise, got {err:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the limit took {elapsed:?} to fire — that is not a limit"
    );
}

/// A loop that only *calls* forever must trip too. Call-heavy code and
/// arithmetic-heavy code are counted by the same trigger, which is why the
/// budget rides on `every_nth_instruction` and not on call events.
#[test]
fn a_runaway_call_loop_is_stopped_too() {
    let mut vm = RealVm::boot_with_instruction_limit(200_000);
    let err = vm
        .eval("local function f() return 1 end while true do f() end")
        .err();
    assert!(err.contains("instruction limit exceeded"), "got {err:?}");
}

/// The budget is per dispatch, not per process — the session has to keep
/// working after one command burns through it.
#[test]
fn the_budget_is_restored_for_the_next_command() {
    let mut vm = RealVm::boot_with_instruction_limit(200_000);

    assert!(vm.eval("while true do end").is_err());
    assert_eq!(vm.eval("return 1 + 1").unwrap(), "2");
    assert!(vm.eval("while true do end").is_err());
    assert_eq!(vm.eval("return 'still here'").unwrap(), "still here");
}

/// Catching the error must not buy another full budget.
///
/// Lua 5.1 has no uncatchable error, so `pcall` will always swallow the one the
/// budget raises. What keeps that from being a hole is that the counter is
/// never reset mid-dispatch: after the first trip, every later count event
/// raises again straight away, so each round costs one `instruction_step`
/// instead of a fresh `limit`. Twenty rounds therefore cost about as much as
/// one — if the budget were refilled they would cost twenty times as much.
#[test]
fn pcall_rounds_do_not_each_get_a_fresh_budget() {
    let mut vm = RealVm::boot_with_instruction_limit(200_000);

    let one = {
        let t = Instant::now();
        assert!(vm.eval("while true do end").is_err());
        t.elapsed()
    };
    let twenty = {
        let t = Instant::now();
        let out = vm.eval(
            "local n = 0 for i = 1, 20 do pcall(function() while true do end end) n = n + 1 end return n",
        );
        assert_eq!(out.unwrap(), "20", "each round should be cut short, not hang");
        t.elapsed()
    };

    assert!(
        twenty < one * 8,
        "20 pcall rounds took {twenty:?} against {one:?} for one — the budget is \
         being refilled on each catch"
    );
}

// KNOWN GAP, deliberately not asserted here because the assertion would be that
// the suite hangs:
//
//     while true do pcall(function() while true do end end) end
//
// still wedges the game thread. `pcall` catches the budget's error, Lua 5.1 has
// no uncatchable one, and every subsequent raise lands back inside the inner
// loop at a fixed offset, so the outer loop is never reached. See the note in
// `debugger::hook::on_event`. The budget stops accidents, not sabotage.

/// Ordinary work has to finish. A limit that fires on a normal command is
/// worse than no limit, because it looks like a bug in the command.
#[test]
fn ordinary_work_finishes_well_inside_a_configured_budget() {
    let mut vm = RealVm::boot_with_instruction_limit(1_000_000);
    assert_eq!(
        vm.eval("local s = 0 for i = 1, 10000 do s = s + i end return s")
            .unwrap(),
        "50005000"
    );
    assert_eq!(
        vm.eval("local t = {} for i = 1, 2000 do t[#t+1] = tostring(i) end return #table.concat(t)")
            .unwrap(),
        "6893"
    );
}

/// Zero means off, and the config comments say so. Nothing should be armed.
#[test]
fn a_limit_of_zero_disables_the_budget() {
    let mut vm = RealVm::boot_with_instruction_limit(0);
    // Bounded, but far past any budget the default config would allow.
    assert_eq!(
        vm.eval("local s = 0 for i = 1, 3000000 do s = s + 1 end return s")
            .unwrap(),
        "3000000"
    );
}

/// `limits.lua_memory_mb` was the other key suspected of being decorative.
/// It is not: mlua accepts `set_memory_limit` on this vendored LuaJIT and
/// enforces it, so the engine applies it. A runaway allocation raises a
/// catchable error and the VM keeps serving.
#[test]
fn the_memory_ceiling_is_enforced_and_survivable() {
    let mut vm = RealVm::boot();

    let err = vm
        .eval("local t = {} for i = 1, 50000000 do t[i] = i end return #t")
        .err();
    assert!(
        err.contains("memory"),
        "expected the allocation to hit the ceiling, got {err:?}"
    );
    assert_eq!(
        vm.eval("return 'still serving'").unwrap(),
        "still serving",
        "the VM must survive a memory error"
    );
}
