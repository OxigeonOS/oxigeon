//! End-to-end coverage for the trace subsystem.
//!
//! Drives a real `Lua` VM with the real hook installed, rather than testing the
//! ring buffer in isolation — the interesting failure modes are all in the
//! arm/disarm lifecycle and the per-dispatch gating.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::prelude::*;
use oxigeon::core::scripting::debugger::{
    self, state::TraceMode, trace, DebugState, HookLocal, InstalledHook,
};

/// Build a VM with the hook wired up exactly as `ScriptEngine::start` does.
struct Harness {
    lua: Lua,
    st: debugger::SharedDebugState,
    hl: Rc<RefCell<HookLocal>>,
    installed: InstalledHook,
}

impl Harness {
    fn new() -> Self {
        trace::clear();
        trace::set_capacities(1000, 50);
        Self {
            lua: Lua::new(),
            st: DebugState::shared(1000, 50),
            hl: Rc::new(RefCell::new(HookLocal::new())),
            installed: InstalledHook::default(),
        }
    }

    /// One dispatch, mirroring the `OnInput` arm of the engine loop.
    fn dispatch(&mut self, session: &str, verb: &str, code: &str) {
        debugger::sync_hook(&self.lua, &self.st, &mut self.installed, &self.hl);
        debugger::set_dispatch_context(&self.st, Some(session));
        debugger::hook::begin_dispatch(&self.hl);

        self.lua.load(code).set_name("@C:/mud/mudlib/cmds/probe.lua").exec().unwrap();

        debugger::hook::end_dispatch(&self.hl, session, verb);
        debugger::set_dispatch_context(&self.st, None);
    }

    fn trace_on(&self, mode: TraceMode, session: &str) {
        self.st.update_trace_config(|c| {
            c.mode = mode;
            c.sessions.insert(session.to_string());
        });
    }
}

const CODE: &str = r#"
local function leaf(n) return n + 1 end
local function mid(n) return leaf(n) + leaf(n) end
local total = 0
for i = 1, 3 do total = total + mid(i) end
return total
"#;

#[test]
fn hook_is_not_installed_until_tracing_is_enabled() {
    let mut h = Harness::new();

    h.dispatch("1", "probe", CODE);
    assert!(!h.installed.is_on(), "hook must stay off when nothing is traced");
    assert_eq!(trace::format_records(100).len(), 0);

    h.trace_on(TraceMode::Calls, "1");
    h.dispatch("1", "probe", CODE);
    assert!(h.installed.is_on(), "enabling tracing should install the hook");
    assert!(!trace::format_records(200).is_empty());

    // ...and disabling it removes the hook again, restoring full speed.
    h.st.update_trace_config(|c| c.mode = TraceMode::Off);
    h.dispatch("1", "probe", CODE);
    assert!(!h.installed.is_on(), "disabling tracing must remove the hook");
    trace::clear();
}

#[test]
fn tracing_is_scoped_to_opted_in_sessions() {
    let mut h = Harness::new();
    h.trace_on(TraceMode::Lines, "1");

    h.dispatch("2", "probe", CODE);
    assert_eq!(
        trace::format_records(100).len(),
        0,
        "a session that did not opt in must not be traced"
    );

    h.dispatch("1", "probe", CODE);
    assert!(!trace::format_records(100).is_empty(), "the opted-in session should be traced");
    trace::clear();
}

#[test]
fn calls_mode_records_entry_and_exit_but_not_lines() {
    let mut h = Harness::new();
    h.trace_on(TraceMode::Calls, "1");
    h.dispatch("1", "probe", CODE);

    let recs = trace::format_records(500);
    assert!(!recs.is_empty());
    assert!(
        recs.iter().all(|r| r.contains('>') || r.contains('<')),
        "calls mode should contain only call/return markers, got {recs:#?}"
    );
    assert!(
        recs.iter().any(|r| r.contains("cmds/probe.lua")),
        "records should carry the shortened chunk path: {recs:#?}"
    );
    trace::clear();
}

#[test]
fn lines_mode_records_strictly_more_than_calls_mode() {
    let mut h = Harness::new();

    h.trace_on(TraceMode::Calls, "1");
    h.dispatch("1", "probe", CODE);
    let calls = trace::format_records(2000).len();
    trace::clear();

    h.trace_on(TraceMode::Lines, "1");
    h.dispatch("1", "probe", CODE);
    let lines = trace::format_records(2000).len();
    trace::clear();

    assert!(lines > calls, "lines mode ({lines}) should record more than calls mode ({calls})");
}

#[test]
fn timing_mode_counts_without_recording() {
    let mut h = Harness::new();
    h.trace_on(TraceMode::Timing, "1");
    h.dispatch("1", "probe", CODE);

    assert_eq!(
        trace::format_records(100).len(),
        0,
        "timing mode must not populate the trace ring"
    );

    let timings = trace::format_timings(10);
    assert_eq!(timings.len(), 2, "header plus one row: {timings:#?}");
    assert!(timings[1].contains("probe"), "verb should be recorded: {timings:#?}");

    // `mid` calls `leaf` twice across three loop iterations, so the counters
    // must be non-zero even though nothing was written to the trace ring.
    trace::with_rings(|r| {
        let t = r.timing.back().unwrap();
        assert!(t.calls >= 9, "expected >=9 calls, got {}", t.calls);
        assert!(t.lines > 0, "expected line counter to advance, got {}", t.lines);
        assert!(t.max_depth >= 2, "expected nesting, got {}", t.max_depth);
    });
    trace::clear();
}

/// Tail calls reuse the caller's frame in Lua 5.1 and emit no matching return,
/// so a Call-minus-Ret counter inflates without bound. Measured against the live
/// mudlib that gave `who` a reported depth of 56. Depth is measured from the
/// real VM stack instead, which is flat across tail recursion.
#[test]
fn tail_recursion_does_not_inflate_reported_depth() {
    let mut h = Harness::new();
    h.trace_on(TraceMode::Timing, "1");
    h.dispatch(
        "1",
        "tails",
        r#"
        local function down(n)
            if n <= 0 then return 0 end
            return down(n - 1)   -- tail call: frame is reused
        end
        return down(200)
        "#,
    );

    trace::with_rings(|r| {
        let t = r.timing.back().unwrap();
        assert!(t.calls >= 200, "expected the calls to be counted, got {}", t.calls);
        assert!(
            t.max_depth < 10,
            "200 tail calls must not report deep nesting, got depth {}",
            t.max_depth
        );
    });
    trace::clear();
}

/// Ordinary recursion *should* show real nesting, so the fix above cannot be
/// achieved by simply reporting a constant.
#[test]
fn ordinary_recursion_still_reports_real_nesting() {
    let mut h = Harness::new();
    h.trace_on(TraceMode::Timing, "1");
    h.dispatch(
        "1",
        "deep",
        r#"
        local function down(n)
            if n <= 0 then return 0 end
            local r = down(n - 1)   -- NOT a tail call
            return r + 1
        end
        return down(20)
        "#,
    );

    trace::with_rings(|r| {
        let t = r.timing.back().unwrap();
        assert!(t.max_depth >= 20, "expected real nesting, got depth {}", t.max_depth);
    });
    trace::clear();
}

#[test]
fn counters_reset_between_dispatches() {
    let mut h = Harness::new();
    h.trace_on(TraceMode::Timing, "1");

    h.dispatch("1", "first", CODE);
    h.dispatch("1", "second", CODE);

    trace::with_rings(|r| {
        assert_eq!(r.timing.len(), 2);
        let a = &r.timing[0];
        let b = &r.timing[1];
        assert_eq!(a.calls, b.calls, "each dispatch should count only its own work");
        assert_eq!(b.verb, "second");
    });
    trace::clear();
}
