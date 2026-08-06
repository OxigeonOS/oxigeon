//! Dispatches suspended at a breakpoint, and the debug requests they answer.
//!
//! Only compiled on the yielding path (Lua 5.3+). Under LuaJIT — and on any
//! build with `stop_the_world` on — a stop blocks the Lua thread inside the hook,
//! so there is nothing to park and every request is serviced from there instead.
//!
//! The rule that shapes all of this: **the frames of a suspended coroutine must
//! not be walked.** They belong to a thread that is not running, and by the time
//! a client asks, this thread is somebody else's command. `hook::park_and_yield`
//! captures what is needed at the moment of the stop, and everything here
//! answers from that capture.
//!
//! # More than one at a time
//!
//! There can be several. A breakpoint on a line a ticker reaches every round
//! parks a *new* dispatch every round, and they are all still suspended — so
//! every request names the stop it is about, and resuming one leaves the others
//! exactly where they were. This began as a single slot and one `Vec::pop`,
//! which meant each stop silently replaced the last: the older one's capture was
//! dropped without ever being released, and the client could no longer ask that
//! stop anything.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mlua::prelude::*;

use super::state::StopId;
use super::{HookLocal, SharedDebugState};

thread_local! {
    /// Whether we are currently holding the collector off. Lua-thread-only, so
    /// a plain `Cell` is enough. Tracked rather than re-asserted so a resume
    /// does not reset the GC debt on every stop.
    static GC_HELD: Cell<bool> = const { Cell::new(false) };
}

/// Stop the collector while anything is parked, and let it go when nothing is.
///
/// **A dispatch parked at a hook yield does not survive a collection.** This is
/// not a policy choice; it is what the runtime does, and it took a live combat
/// round to find:
///
/// ```text
/// COMBAT_D: attack failed: mudlib/lib/mobile.lua:185:
///   attempt to index a nil value (local 'self')
/// ```
///
/// on the line *after* one that used `self` perfectly happily.
///
/// `luaD_hook` raises `L->top` to `ci->top` for the duration of a hook —
/// "protect entire activation register", in its own words — and puts the low,
/// mid-instruction value back before returning. A hook that *yields* is no
/// exception: `luaG_traceexec` throws `LUA_YIELD` only after `luaD_hook` has
/// restored it. So a thread suspended at a hook yield sits there with `top`
/// **below** the live registers of the frame it stopped in, and `lgc.c` takes
/// that at face value:
///
/// ```c
///   for (; o < th->top.p; o++)  markvalue(g, s2v(o));    /* live */
///   ...
///   for (o = th->top.p; o < th->stack_last.p + EXTRA_STACK; o++)
///     setnilvalue(s2v(o));                               /* "dead" slice */
/// ```
///
/// The first atomic phase to run while the dispatch is parked nils its
/// parameters and locals — and `luaD_shrinkstack`, on the line above, can cut
/// the stack out from under it as well. A normal `coroutine.yield` is
/// unaffected, because it suspends at a C boundary with `top` above everything
/// live; this is specific to yielding from a hook, which is exactly what a
/// breakpoint does.
///
/// Nothing outside the VM can raise a suspended thread's `top` — `lua_settop`
/// fills the slots it passes with nil, which is the very damage being avoided —
/// so holding the collector off for the parked window is the available fix.
///
/// The cost is that a stop stops collecting, for as long as the stop lasts. That
/// is bounded by `auto_continue_secs` and only reachable with a debug client
/// attached, which is the right trade against a debugger that corrupts the
/// program it is inspecting. `collectgarbage` is not exposed to game code, so
/// nothing in the mudlib can force a cycle in the meantime.
///
/// See `tests/debug_parked_gc.rs`.
fn hold_collector(lua: &Lua, parked: &[ParkedDispatch]) {
    let want = !parked.is_empty();
    GC_HELD.with(|held| {
        if held.get() == want {
            return;
        }
        if want {
            lua.gc_stop();
        } else {
            lua.gc_restart();
        }
        held.set(want);
    });
}

/// Suspend a dispatch, with the collector held off for as long as it is.
///
/// The one way to add to `parked`: the hold has to be in place before any other
/// Lua runs, and the push is the moment that becomes true.
pub fn park(lua: &Lua, parked: &mut Vec<ParkedDispatch>, entry: ParkedDispatch) {
    parked.push(entry);
    hold_collector(lua, parked);
}

/// A dispatch suspended at a breakpoint.
pub struct ParkedDispatch {
    /// The stop this coroutine belongs to. Also its DAP `threadId`.
    pub id: StopId,
    /// Whose command this is. Empty for a timer, which nobody is waiting on.
    pub session: String,
    /// Kept so the timing ring still gets its row when the dispatch finishes.
    pub verb: String,
    /// Whether it runs with the engine's own identity.
    ///
    /// A tick has no player behind it, so gated efuns a daemon calls are allowed
    /// only while `enter_system_dispatch`'s guard is alive. That guard is a
    /// stack value, and parking unwinds the stack it was on — so it has to be
    /// re-established around every resume, or the second half of a ticker that
    /// stopped at a breakpoint is denied everything the first half was allowed.
    pub system: bool,
    pub thread: LuaThread,
}

/// Answer the debug client while dispatches are suspended, and resume them when
/// told to.
///
/// Everything here runs on the Lua thread between commands, which is what makes
/// it safe to touch the VM. What it must *not* do is walk a suspended stack:
/// those frames belong to coroutines that are not running. `park_and_yield`
/// captured them at the moment of each stop, and this answers from that.
pub fn serve(
    lua: &Lua,
    st: &SharedDebugState,
    hl: &Rc<RefCell<HookLocal>>,
    parked: &mut Vec<ParkedDispatch>,
) {
    use super::state::VmRequest;

    // Taken for the duration: servicing calls back into the hook state, and
    // holding a borrow across that would panic.
    let Some(rx) = super::hook::take_channel(hl) else { return };

    while let Ok(req) = rx.try_recv() {
        match req {
            VmRequest::StackTrace { stop, levels, reply } => {
                let frames = st
                    .parked_stop(stop)
                    .map(|p| p.frames.into_iter().take(levels).collect())
                    .unwrap_or_default();
                let _ = reply.send(frames);
            }
            VmRequest::Scopes { stop, frame, reply } => {
                let scopes = st
                    .parked_stop(stop)
                    .map(|p| super::introspect::capture_scopes(lua, p.capture, frame))
                    .unwrap_or_default();
                let _ = reply.send(scopes);
            }
            VmRequest::Variables { var_ref, reply } => {
                // Handles are values, not stack positions, so this needs no
                // capture id — `introspect.lua` still owns them.
                let _ = reply.send(super::introspect::variables(lua, var_ref));
            }
            VmRequest::Evaluate { stop, frame, expr, reply } => {
                let answer = match st.parked_stop(stop) {
                    Some(p) => super::introspect::capture_evaluate(lua, p.capture, frame, &expr),
                    None => Err("that stop has already resumed".to_string()),
                };
                let _ = reply.send(answer);
            }
            VmRequest::Resume { stop, kind } => {
                super::hook::put_channel(hl, rx);
                resume(lua, st, hl, parked, stop, Some(kind));
                return;
            }
            VmRequest::Detach => {
                // Detaching must not leave a player's command suspended for
                // ever: run every parked dispatch to completion.
                super::hook::put_channel(hl, rx);
                resume_all(lua, st, hl, parked);
                return;
            }
        }
    }

    super::hook::put_channel(hl, rx);
}

/// Run every parked dispatch to completion if any stop has outlived
/// `auto_continue_secs`.
///
/// The blocking path has always had this valve — `enter_pause` bounds its wait —
/// and the yielding path needs it for a different reason. A stop here does not
/// wedge the server, so nobody would notice; what it strands is the *player*
/// whose command it was, sitting at a dead prompt because their editor crashed.
pub fn auto_continue(
    lua: &Lua,
    st: &SharedDebugState,
    hl: &Rc<RefCell<HookLocal>>,
    parked: &mut Vec<ParkedDispatch>,
) {
    if parked.is_empty() {
        return;
    }
    let overdue = parked.iter().any(|d| {
        st.parked_stop(d.id)
            .is_some_and(|p| p.since.elapsed() >= st.auto_continue)
    });
    if !overdue {
        return;
    }
    tracing::warn!(
        "debugger: no client response after {:?} — resuming the suspended dispatch(es)",
        st.auto_continue
    );
    resume_all(lua, st, hl, parked);
}

/// Resume everything, newest first. Used by detach and the auto-continue valve.
fn resume_all(
    lua: &Lua,
    st: &SharedDebugState,
    hl: &Rc<RefCell<HookLocal>>,
    parked: &mut Vec<ParkedDispatch>,
) {
    use super::state::ResumeKind;
    while let Some(id) = parked.last().map(|d| d.id) {
        let before = parked.len();
        resume(lua, st, hl, parked, id, Some(ResumeKind::Continue));
        // A dispatch that stops again on its way out stays parked; without this
        // the loop would spin on it for ever.
        if parked.len() >= before {
            break;
        }
    }
}

/// Resume one suspended dispatch, named by its stop id.
pub fn resume(
    lua: &Lua,
    st: &SharedDebugState,
    hl: &Rc<RefCell<HookLocal>>,
    parked: &mut Vec<ParkedDispatch>,
    stop: StopId,
    kind: Option<super::state::ResumeKind>,
) {
    let Some(at) = parked.iter().position(|d| d.id == stop) else {
        // Already finished, or never existed. A client that resumed twice is
        // not an error worth reporting to the game.
        tracing::debug!("debugger: resume for stop {stop}, which is not parked");
        return;
    };
    let entry = parked.remove(at);

    // Bank the frozen interval and drop this stop's capture before anything runs
    // again. Releasing *by id* is what keeps a capture from outliving its stop —
    // the reason the registry is a map rather than one slot.
    let mut stop_depth = 0u16;
    if let Some(p) = st.unpark(stop) {
        st.add_paused(p.since.elapsed());
        stop_depth = p.depth;
        super::introspect::release(lua, p.capture);
    }
    // Handles are per-stop; a variables pane holding one from a stop that has
    // resumed would be showing a frame that no longer exists.
    super::introspect::reset(lua);

    if let Some(kind) = kind {
        super::hook::arm_step(hl, kind, stop_depth);
    }

    st.emit(super::state::DebugEventMsg::Continued { stop, world: false });

    // The session is whose command this is, so a stop after the resume reports
    // the right one.
    let session = (!entry.session.is_empty()).then_some(entry.session.as_str());
    super::set_dispatch_context(st, session);
    super::set_dispatch_label(&entry.verb);
    crate::core::scripting::efuns::set_current_session(session.map(str::to_string));
    let _system = entry
        .system
        .then(crate::core::scripting::efuns::enter_system_dispatch);

    match entry.thread.resume::<()>(()) {
        Ok(()) => {
            if entry.thread.status() == mlua::ThreadStatus::Resumable {
                // Stopped again — a step, or the next breakpoint. That is a
                // *new* stop with a new id, which `park_and_yield` has already
                // registered; pair the coroutine with it.
                let mut entry = entry;
                if let Some(new_id) = super::hook::take_parked_id(hl) {
                    entry.id = new_id;
                }
                park(lua, parked, entry);
                return;
            }
            super::hook::end_dispatch(hl, &entry.session, &entry.verb);
        }
        Err(e) => {
            tracing::error!("Lua error resuming a suspended dispatch: {}", e);
        }
    }

    // This one is gone; if it was the last, the collector can run again.
    hold_collector(lua, parked);

    super::set_dispatch_context(st, None);
    super::set_dispatch_label("");
    crate::core::scripting::efuns::set_current_session(None);
}

/// Run one dispatch to completion, serving the debug client while it is
/// suspended.
///
/// This is the *blocking* shape, for callers whose only job is that one
/// dispatch — the test harnesses in `tests/dap_attach.rs` and friends, which
/// stand a bare VM next to the adapter and want the chunk to finish. The engine
/// deliberately does not use it: the whole point there is to return to the loop
/// and keep serving other players.
///
/// Terminates because a stop is always ended by something — a client resume, a
/// detach, or `auto_continue_secs` — and `deadline` bounds the wait regardless.
pub fn run_blocking(
    lua: &Lua,
    st: &SharedDebugState,
    hl: &Rc<RefCell<HookLocal>>,
    thread: LuaThread,
    session: &str,
) {
    super::arm_thread(&thread, st, hl);

    if let Err(e) = thread.resume::<()>(()) {
        tracing::error!("dispatch failed: {}", e);
        return;
    }
    if thread.status() != mlua::ThreadStatus::Resumable {
        return;
    }

    // Through `park`, not a bare `vec![]`: `serve` below evaluates expressions
    // and builds variable panes, which allocates, and a collection while this
    // dispatch is suspended would take its frame apart. See `hold_collector`.
    let mut parked = Vec::new();
    park(
        lua,
        &mut parked,
        ParkedDispatch {
            id: super::hook::take_parked_id(hl).unwrap_or(super::state::WORLD_STOP),
            session: session.to_string(),
            verb: String::new(),
            system: false,
            thread,
        },
    );

    let deadline = std::time::Instant::now() + st.auto_continue;
    while !parked.is_empty() {
        serve(lua, st, hl, &mut parked);
        if parked.is_empty() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            tracing::warn!(
                "debugger: no client response after {:?} — resuming so the dispatch \
                 is not abandoned",
                st.auto_continue
            );
            resume_all(lua, st, hl, &mut parked);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
