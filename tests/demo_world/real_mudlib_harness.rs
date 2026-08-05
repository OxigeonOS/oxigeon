//! `RealVm::boot_real_mudlib` runs the repository's actual `mudlib/` and
//! `game/` rather than a throwaway probe mudlib. `benches/dispatch.rs` is
//! built on it, so it needs to be known-good independently of the benchmark —
//! a benchmark that silently measured a half-booted game would be worse than
//! no benchmark.
//!
//! It doubles as a smoke test of the whole stack: daemon load, the real login
//! flow, command dispatch, the world, and the prompt.


use crate::common::RealVm;

/// Booting is most of the test: `boot_real_mudlib` asserts internally that the
/// login flow reached the game, so getting here at all means the daemons
/// loaded, the account was created off-thread, a character was made, and the
/// world placed it.
#[test]
fn the_real_mudlib_boots_and_logs_a_session_in() {
    let mut vm = RealVm::boot_real_mudlib(0);
    let out = vm.command("look");
    assert!(
        out.contains("Entrance to the Workshop"),
        "expected the start room's description, got: {out:?}"
    );
}

/// The commands the benchmark measures all have to work, and each has to end
/// with a prompt — which is what `command` waits on. A command that never
/// rendered one would hang the benchmark rather than fail it.
#[test]
fn every_benchmarked_command_completes() {
    let mut vm = RealVm::boot_real_mudlib(0);

    for (verb, expected) in [
        ("look", "Entrance to the Workshop"),
        ("who", "benchuser"),
        ("inventory", "carrying"),
        // Admin-only, and the first account is auto-promoted — so this also
        // proves the permission cache was populated at enter_game.
        ("mudstatus", "Uptime"),
    ] {
        let out = vm.command(verb);
        assert!(
            out.contains(expected),
            "`{verb}` should mention {expected:?}, got: {out:?}"
        );
    }
}

/// An unknown verb must still come back, or a typo in a benchmark id would
/// look like a hang.
#[test]
fn an_unknown_command_still_returns() {
    let mut vm = RealVm::boot_real_mudlib(0);
    let out = vm.command("xyzzy_no_such_command");
    assert!(out.contains("Unknown command"), "got: {out:?}");
}

/// The same VM has to survive being driven hard — the benchmark will dispatch
/// thousands of times against one instance.
#[test]
fn the_session_survives_repeated_dispatch() {
    let mut vm = RealVm::boot_real_mudlib(0);
    for _ in 0..200 {
        assert!(vm.command("look").contains("Entrance"));
    }
    assert!(vm.command("who").contains("benchuser"), "still alive at the end");
}

/// The other half of the 2x2 the benchmark measures. With a limit configured
/// the engine turns the JIT off, and the real mudlib has to keep working —
/// booting the whole daemon tree is by far the biggest chunk of Lua this
/// project runs, so if a budget were going to trip on legitimate work, here is
/// where it would show.
#[test]
fn the_real_mudlib_works_with_the_instruction_budget_armed() {
    let mut vm = RealVm::boot_real_mudlib(1_000_000);
    assert!(vm.command("look").contains("Entrance to the Workshop"));
    assert!(vm.command("who").contains("benchuser"));
}
