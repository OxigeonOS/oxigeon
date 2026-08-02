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
}

/// Tracks whether the hook is currently installed on the VM.
#[derive(Default)]
pub struct InstalledHook {
    on: bool,
}

impl InstalledHook {
    pub fn is_on(&self) -> bool {
        self.on
    }
}

/// Install or remove the hook to match [`DebugState`].
///
/// Must only be called from the Lua thread, and never from inside the hook
/// itself — mlua holds an `Rc` clone of the callback for the duration of
/// `hook_proc`, so re-entering `set_hook` there is not safe.
///
/// The installed trigger mask is *constant* while armed; what the callback
/// actually does is gated by `DebugState::want`. A mask that varied with the
/// breakpoint or step state would have to be swapped from inside the hook.
pub fn sync_hook(
    lua: &Lua,
    st: &SharedDebugState,
    installed: &mut InstalledHook,
    hl: &Rc<RefCell<HookLocal>>,
) {
    let should_be_on = st.armed.load(Ordering::Relaxed) && st.want.load(Ordering::Relaxed) != 0;

    if should_be_on && !installed.on {
        let st2 = st.clone();
        let hl2 = hl.clone();
        lua.set_hook(
            HookTriggers {
                on_calls: true,
                on_returns: true,
                every_line: true,
                every_nth_instruction: None,
            },
            move |lua, dbg| hook::on_event(lua, dbg, &st2, &hl2),
        );
        installed.on = true;
        tracing::debug!("debugger: hook installed");
    } else if !should_be_on && installed.on {
        lua.remove_hook();
        installed.on = false;
        tracing::debug!("debugger: hook removed");
    }
}
