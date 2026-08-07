//! A dispatch parked at a hook yield must survive the collector.
//!
//! Nothing here applies under LuaJIT: `VmState::Yield` from a hook raises there,
//! so a stop blocks the Lua thread instead of parking and no frame is ever left
//! suspended for a collection to find. The whole file is gated rather than each
//! test, because the shape it probes does not exist on that runtime.
//!
//! Found in a live combat round, with a breakpoint held open until the
//! auto-continue valve fired:
//!
//! ```text
//! COMBAT_D: attack failed: mudlib/lib/mobile.lua:185:
//!   attempt to index a nil value (local 'self')
//! ```
//!
//! on the line *after* one that had used `self` perfectly happily — and only
//! ever while debugging.
//!
//! The first test is a spike against the bare runtime: it records the mechanism
//! and shows that holding the collector off is what fixes it. The second drives
//! oxigeon's own park path and is the regression guard.
//!
//! # What the spike records
//!
//! `luaD_hook` raises `L->top` to `ci->top` for the duration of a hook
//! ("protect entire activation register") and puts it back afterwards. A hook
//! that *yields* is no exception: `luaG_traceexec` throws `LUA_YIELD` only after
//! `luaD_hook` has restored the low, mid-instruction `top`. So a thread
//! suspended at a hook yield sits there with `top` **below** the live registers
//! of the frame it stopped in.
//!
//! `traversethread` in `lgc.c` believes that:
//!
//! ```c
//!   for (; o < th->top.p; o++)  markvalue(g, s2v(o));   /* live */
//!   ...
//!   for (o = th->top.p; o < th->stack_last.p + EXTRA_STACK; o++)
//!     setnilvalue(s2v(o));                              /* "dead" slice */
//! ```
//!
//! so the first atomic phase that runs while the dispatch is parked nils the
//! frame's parameters and locals. `self` goes nil in the middle of a method that
//! was using it a line earlier.
//!
//! A normal `coroutine.yield` is unaffected: it suspends at a C boundary with
//! `top` above everything live. This is specific to yielding from a hook, which
//! is exactly what the breakpoint path does.

#![cfg(not(feature = "luajit"))]

use mlua::prelude::*;
use mlua::{HookTriggers, StdLib, VmState};
use std::cell::RefCell;
use std::rc::Rc;

/// What happens between the park and the resume.
#[derive(Clone, Copy)]
enum Between {
    /// Resume straight away, as a test harness does.
    Nothing,
    /// Other players' commands run, allocating — as the live engine does.
    OtherCode,
    /// A full collection, the same thing the allocations above eventually cause.
    FullGc,
    /// Other code runs, but the collector is paused for the parked window.
    OtherCodeGcPaused,
}

fn probe(between: Between) -> String {
    let lua = unsafe {
        Lua::unsafe_new_with(StdLib::ALL_SAFE | StdLib::DEBUG, LuaOptions::default())
    };

    // Shaped like `Mobile:take_damage`: the stop is on the line that assigns
    // `_killed_by`, and the line after it indexes `self` again.
    let chunk = r#"
        local Mobile = {}
        Mobile.__index = Mobile

        function Mobile:take_damage(amount, opts)
            local was_alive = self.hp > 0
            self.hp = self.hp - amount
            if was_alive and self.hp <= 0 then
                self.killed_by = opts.attacker      -- STOP HERE
                return "killed " .. self.name .. " with " .. self.killed_by
            end
            return "alive"
        end

        local mob = setmetatable({ hp = 5, name = "rat" }, Mobile)
        return coroutine.create(function()
            RESULT = mob:take_damage(10, { attacker = "player" })
        end)
    "#;

    // Located rather than counted, so editing the chunk cannot silently move the
    // breakpoint off the line it is meant to be on.
    let stop_line = chunk
        .lines()
        .position(|l| l.contains("-- STOP HERE"))
        .expect("the marked line must exist")
        + 1;

    let co: LuaThread = lua
        .load(chunk)
        .set_name("@parkspike.lua")
        .call(())
        .expect("chunk must build the coroutine");

    // PUC hooks are per-thread, so the coroutine is armed rather than the main
    // state — the same reason `debugger::arm_thread` exists.
    let fired = Rc::new(RefCell::new(false));
    let flag = fired.clone();
    co.set_hook(HookTriggers::new().every_line(), move |_lua, d| {
        if d.current_line() != Some(stop_line) || *flag.borrow() {
            return Ok(VmState::Continue);
        }
        *flag.borrow_mut() = true;
        Ok(VmState::Yield)
    })
    .expect("hook must install");

    if let Err(e) = co.resume::<()>(()) {
        return format!("first resume failed: {e}");
    }
    if co.status() != mlua::ThreadStatus::Resumable {
        return "never parked — the hook yield was not honoured".to_string();
    }

    let run = |src: &str| {
        lua.load(src)
            .exec()
            .unwrap_or_else(|e| panic!("probe setup failed: {e}"))
    };
    let churn = "local t = {} for i = 1, 5000 do t[i] = { i = i, s = tostring(i) } end";
    match between {
        Between::Nothing => {}
        Between::OtherCode => run(churn),
        Between::FullGc => run("collectgarbage('collect')"),
        Between::OtherCodeGcPaused => {
            run("collectgarbage('stop')");
            run(churn);
            run("collectgarbage('restart')");
        }
    }

    match co.resume::<()>(()) {
        Ok(()) => lua
            .globals()
            .get::<Option<String>>("RESULT")
            .ok()
            .flatten()
            .unwrap_or_else(|| "<no result>".into()),
        // Trimmed to the first line: the traceback below it is noise here.
        Err(e) => format!("ERROR: {}", e.to_string().lines().next().unwrap_or("")),
    }
}

#[test]
fn a_parked_frame_loses_its_locals_to_the_collector() {
    let immediate = probe(Between::Nothing);
    let other_code = probe(Between::OtherCode);
    let full_gc = probe(Between::FullGc);
    let gc_paused = probe(Between::OtherCodeGcPaused);

    println!("--- resuming a dispatch parked at a hook yield ---");
    println!("  resumed immediately        : {immediate}");
    println!("  after other code ran       : {other_code}");
    println!("  after a full collection    : {full_gc}");
    println!("  other code, collector off  : {gc_paused}");

    const OK: &str = "killed rat with player";

    // Nothing wrong with the park itself: a harness that resumes straight away
    // — which is every test in `tests/dap_attach.rs` — sees a healthy frame.
    // That is why this has been green the whole time.
    assert_eq!(immediate, OK, "the park itself is not the problem");

    // But the live engine goes back to serving other players, and the first
    // atomic phase after that takes the frame apart.
    assert!(
        other_code.contains("attempt to index a nil value"),
        "expected the parked frame to be wiped by ordinary allocation: {other_code}"
    );
    assert!(
        full_gc.contains("attempt to index a nil value"),
        "expected a full collection to wipe the parked frame: {full_gc}"
    );

    // Holding the collector off for the parked window is enough to keep it.
    assert_eq!(
        gc_paused, OK,
        "pausing the collector while a dispatch is parked should preserve it"
    );
}

// ─── the regression guard, through oxigeon's own park path ───────────────────

/// A dispatch stopped at a breakpoint, resumed after other players have been
/// served, still has the frame it stopped in.
///
/// Goes through `debugger::parked` rather than reproducing its shape, so this
/// fails if the collector hold is ever dropped from the park lifecycle. The
/// churn between the park and the resume is what other players' commands are:
/// ordinary allocation, which is all it takes to reach an atomic phase.
#[test]
fn a_parked_dispatch_survives_other_players_commands() {
    use oxigeon::core::scripting::debugger::{self, paths, DebugState, HookLocal};
    use std::sync::atomic::Ordering;

    const CHUNK: &str = "\
local function take_damage(self, opts)
    local was_alive = self.hp > 0
    self.hp = self.hp - 10
    if was_alive and self.hp <= 0 then
        self.killed_by = opts.attacker
        return 'killed ' .. self.name .. ' with ' .. self.killed_by
    end
    return 'alive'
end
RESULT = take_damage({ hp = 5, name = 'rat' }, { attacker = 'player' })
";
    // The line that assigns `killed_by` — the shape of `mobile.lua:185`.
    const STOP_LINE: u32 = 5;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("probe.lua");
    std::fs::write(&path, CHUNK).unwrap();
    let chunk_name = paths::chunk_name(&path);

    let st = DebugState::shared(64, 64);
    // The yielding path is the one under test: a stop holds one dispatch and
    // lets the rest of the game carry on, which is what makes a collection
    // possible while a frame is suspended.
    st.stop_the_world.store(false, Ordering::Relaxed);
    st.clients.store(1, Ordering::Relaxed);
    st.republish();
    st.set_breakpoints(
        paths::chunk_key(&chunk_name).expect("the probe path must normalize"),
        &[(STOP_LINE, debugger::state::BreakpointSpec::default())],
    );

    let lua = unsafe {
        Lua::unsafe_new_with(StdLib::ALL_SAFE | StdLib::DEBUG, LuaOptions::default())
    };
    debugger::introspect::hide_debug_library(&lua).unwrap();
    debugger::introspect::load_helper(&lua).unwrap();

    let hl = Rc::new(RefCell::new(HookLocal::new()));
    if let Some(rx) = st.take_vm_rx() {
        hl.borrow_mut().attach_channel(rx);
    }

    let f = lua
        .load(CHUNK)
        .set_name(&chunk_name)
        .into_function()
        .unwrap();
    let thread = lua.create_thread(f).unwrap();
    debugger::arm_thread(&thread, &st, &hl);

    thread.resume::<()>(()).expect("the dispatch must start");
    assert_eq!(
        thread.status(),
        mlua::ThreadStatus::Resumable,
        "the breakpoint did not park the dispatch — this test asserts nothing"
    );

    let mut parked = Vec::new();
    debugger::parked::park(
        &lua,
        &mut parked,
        debugger::parked::ParkedDispatch {
            id: debugger::hook::take_parked_id(&hl).unwrap_or(debugger::state::WORLD_STOP),
            session: String::new(),
            verb: "attack".to_string(),
            system: false,
            thread,
        },
    );
    assert!(
        !lua.gc_is_running(),
        "the collector must be held off while a dispatch is parked"
    );

    // The rest of the game carries on: other commands, other allocation.
    lua.load("local t = {} for i = 1, 20000 do t[i] = { i = i, s = tostring(i) } end")
        .exec()
        .unwrap();

    let stop = parked[0].id;
    debugger::parked::resume(
        &lua,
        &st,
        &hl,
        &mut parked,
        stop,
        Some(debugger::state::ResumeKind::Continue),
    );

    assert!(parked.is_empty(), "the dispatch should have run to completion");
    assert_eq!(
        lua.globals().get::<Option<String>>("RESULT").unwrap().as_deref(),
        Some("killed rat with player"),
        "the resumed dispatch lost the frame it stopped in"
    );
    assert!(
        lua.gc_is_running(),
        "the collector must be let go once nothing is parked"
    );
}
