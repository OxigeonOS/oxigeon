//! Live Lua debugging support: execution tracing and (later) a VS Code DAP adapter.
//!
//! The whole mudlib runs on one thread inside one `Lua` VM ([`super::engine`]),
//! and mlua's hook API offers no yield on LuaJIT — returning `VmState::Yield`
//! there raises `"attempt to yield from a hook"`. Pausing therefore means
//! *blocking that thread*, and every operation that touches the VM has to be
//! serviced from inside the hook callback while it is blocked.
//!
//! The in-game `trace` command and the DAP adapter are deliberately one
//! subsystem. They share the hook, the chunk-name mapping, and the ring buffer —
//! and they have to, because a `lua_State` has exactly one hook slot.

pub mod dap;
pub mod efuns;
pub mod hook;
pub mod introspect;
#[cfg(not(feature = "luajit"))]
pub mod parked;
pub mod paths;
pub mod state;
pub mod trace;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::Ordering;

use mlua::prelude::*;
use mlua::HookTriggers;

pub use hook::HookLocal;
pub use state::{DebugState, SharedDebugState, TraceConfig, TraceMode};

thread_local! {
    /// Whether the dispatch currently running should be traced.
    ///
    /// Computed once per dispatch rather than read per line: resolving the
    /// session against `TraceConfig` on every hook event would mean a `RefCell`
    /// borrow plus a `String` clone thousands of times per command.
    static DISPATCH_TRACED: Cell<bool> = const { Cell::new(false) };
    /// Which session the dispatch in flight belongs to, so a stop can say whose
    /// it is. Set alongside `DISPATCH_TRACED`.
    static DISPATCH_SESSION: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
    /// What the dispatch in flight *is*, for a client's thread list: the verb a
    /// player typed, or `timer:<id>`. A suspended dispatch is only meaningfully
    /// pickable out of a list if it is named, and "thread 1" names nothing.
    static DISPATCH_LABEL: std::cell::RefCell<String> =
        const { std::cell::RefCell::new(String::new()) };
}

/// The session whose dispatch is running, if any.
pub fn current_dispatch_session() -> Option<String> {
    DISPATCH_SESSION.with(|c| c.borrow().clone())
}

/// What the dispatch in flight is, for naming a stop.
pub fn current_dispatch_label() -> String {
    DISPATCH_LABEL.with(|c| c.borrow().clone())
}

/// Name the dispatch about to run. Cleared with `""` when it ends.
pub fn set_dispatch_label(label: &str) {
    DISPATCH_LABEL.with(|c| *c.borrow_mut() = label.to_string());
}

#[inline]
pub(crate) fn dispatch_is_traced() -> bool {
    DISPATCH_TRACED.with(Cell::get)
}

/// Record whether the dispatch about to run is in scope for tracing.
///
/// Call alongside `efuns::set_current_session`, with `None` when the dispatch ends.
pub fn set_dispatch_context(st: &SharedDebugState, session_id: Option<&str>) {
    let traced = st.armed.load(Ordering::Relaxed) && st.trace_config().covers(session_id);
    DISPATCH_TRACED.with(|c| c.set(traced));
    DISPATCH_SESSION.with(|c| *c.borrow_mut() = session_id.map(str::to_string));
}

/// Which triggers the VM's single hook slot should be armed with.
///
/// The three are kept apart because their costs differ by orders of magnitude,
/// measured on this build over a string/table workload shaped like a MUD
/// command (5-run average, JIT already off):
///
/// ```text
///   no hook                    15.0 ms
///   calls + returns + count    15.4 ms    +2%
///   every_line                 44.6 ms    +197%
/// ```
///
/// Those figures are from a synthetic workload; `benches/dispatch.rs` puts the
/// count trigger at 1-3% of a real command dispatch. Either way the ordering
/// is what matters: `every_line` costs multiples, the count trigger costs
/// percent.
///
/// So tracing, breakpoints and stepping get `every_line`, and the instruction
/// budget — which is on by default — does not. Folding them into one mask
/// would make a configured instruction limit cost as much as a permanent line
/// trace.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct HookShape {
    /// Function entry and exit.
    calls: bool,
    /// Every executed source line. The expensive one.
    lines: bool,
    /// Instruction-count interval.
    every_nth: Option<u32>,
}

impl HookShape {
    fn for_state(st: &SharedDebugState) -> Self {
        if !st.armed.load(Ordering::Relaxed) {
            return Self::default();
        }
        let wants = st.want.load(Ordering::Relaxed);
        let watching = wants & state::want::NEEDS_EVENTS != 0;
        let budget = wants & state::want::BUDGET != 0;
        Self {
            // The budget gets the count trigger and nothing else. Adding
            // `on_calls`/`on_returns` was tried and made things *worse*: with
            // CALL|RET|COUNT in the mask, LuaJIT stopped delivering count
            // events inside a loop that makes no calls, and a plain
            // `while true do end` became uninterruptible again. Count alone
            // interrupts every runaway loop in `tests/instruction_limit.rs`.
            calls: watching,
            lines: watching,
            every_nth: budget.then_some(st.instruction_step),
        }
    }

    fn is_off(self) -> bool {
        !self.calls && !self.lines && self.every_nth.is_none()
    }

    fn triggers(self) -> HookTriggers {
        HookTriggers {
            on_calls: self.calls,
            on_returns: self.calls,
            every_line: self.lines,
            every_nth_instruction: self.every_nth,
        }
    }
}

/// Tracks the trigger mask currently installed on the VM.
#[derive(Default)]
pub struct InstalledHook {
    shape: HookShape,
}

impl InstalledHook {
    pub fn is_on(&self) -> bool {
        !self.shape.is_off()
    }
}

/// Give a dispatch coroutine the same hook the main thread has.
///
/// **PUC Lua hooks are per-thread.** `lua_sethook` sets them on one
/// `lua_State`, and a coroutine created from it inherits nothing — so a
/// dispatch running on its own thread would have no breakpoints, no stepping,
/// and, worse, no instruction budget: `while true do end` would simply wedge
/// the game with nothing to interrupt it.
///
/// This is why the coroutine path arms every thread it creates. It found the
/// bug the hard way — the budget tests hung.
#[cfg(not(feature = "luajit"))]
pub fn arm_thread(
    thread: &LuaThread,
    st: &SharedDebugState,
    hl: &Rc<RefCell<HookLocal>>,
) {
    let shape = HookShape::for_state(st);
    if shape.is_off() {
        return;
    }
    let st2 = st.clone();
    let hl2 = hl.clone();
    if let Err(e) = thread.set_hook(shape.triggers(), move |lua, dbg| {
        hook::on_event(lua, dbg, &st2, &hl2)
    }) {
        tracing::error!("debugger: could not arm the dispatch coroutine: {}", e);
    }
}

/// Install, replace or remove the hook to match [`DebugState`].
///
/// Must only be called from the Lua thread, and never from inside the hook
/// itself — mlua holds an `Rc` clone of the callback for the duration of
/// `hook_proc`, so re-entering `set_hook` there is not safe. This runs from the
/// engine's command loop, between dispatches, which is the one place that
/// holds.
///
/// The mask is therefore constant *while a dispatch runs*, but not for the life
/// of the process: attaching a debugger to a VM that only had the instruction
/// budget armed widens it here, and detaching narrows it again.
pub fn sync_hook(
    lua: &Lua,
    st: &SharedDebugState,
    installed: &mut InstalledHook,
    hl: &Rc<RefCell<HookLocal>>,
) {
    let shape = HookShape::for_state(st);
    if shape == installed.shape {
        return;
    }

    if shape.is_off() {
        lua.remove_hook();
        tracing::debug!("debugger: hook removed");
    } else {
        let st2 = st.clone();
        let hl2 = hl.clone();
        // mlua 0.11 reports failure rather than panicking. A hook that did not
        // install is a debugger that never stops and a budget that never fires,
        // and the old signature let that pass silently.
        if let Err(e) = lua.set_hook(shape.triggers(), move |lua, dbg| {
            hook::on_event(lua, dbg, &st2, &hl2)
        }) {
            tracing::error!("debugger: could not install the hook: {}", e);
            installed.shape = HookShape::default();
            return;
        }
        tracing::debug!("debugger: hook installed ({:?})", shape);
    }
    installed.shape = shape;
}
