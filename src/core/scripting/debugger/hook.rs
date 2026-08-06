//! The single Lua hook callback.
//!
//! There is exactly one hook slot per `lua_State`, so tracing, breakpoints, and
//! stepping all have to share this one callback — that is the structural reason
//! `trace` cannot have a hook of its own.
//!
//! The callback must always return [`VmState::Continue`]. Returning `Yield`
//! under the `luajit` feature makes mlua's `hook_proc` raise
//! `"attempt to yield from a hook"`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use crate::core::lock::MutexExt;

use mlua::prelude::*;
use mlua::{Debug as LuaDebug, DebugEvent, VmState};

use super::introspect::Condition;
use super::state::{
    want, BreakpointSpec, DebugEventMsg, Frame, ResumeKind, SharedDebugState, StopReason,
    TraceMode, VmRequest,
};
use super::trace::{self, CommandTiming, TraceKind, TraceRecord};

/// Mutable hook state owned by the Lua thread.
///
/// `Lua::set_hook` takes an `Fn`, not an `FnMut`, so this lives in an
/// `Rc<RefCell<..>>` created outside the closure. Keeping it outside also means
/// the interning table and counters survive a disarm/re-arm cycle.
pub struct HookLocal {
    /// Last `DebugState::generation` this cache was refreshed at.
    gen_seen: u64,
    mode: TraceMode,
    wants: u8,

    /// Interned chunk names and function names, so a traced line allocates nothing.
    intern: HashMap<String, Arc<str>>,

    /// Current call depth. Re-measured from the real VM stack on every call
    /// event rather than counted incrementally: a Call/Ret counter drifts badly
    /// in practice because LuaJIT reuses the frame for tail calls (no matching
    /// return) and C functions do not report returns symmetrically. Measured
    /// against the live mudlib, counting gave a depth of 56 for `who`.
    depth: u16,
    /// Stack depth at the first event of the current dispatch, subtracted from
    /// `depth` so indentation starts at zero rather than at the engine's frames.
    base_depth: Option<u16>,

    // per-dispatch counters
    t0: Instant,
    lines: u32,
    calls: u32,
    max_depth: u16,

    // ── debugging ────────────────────────────────────────────────────────
    /// Breakpoints, copied from `DebugState` on generation change.
    bps: HashMap<super::paths::NormPath, HashMap<u32, BreakpointSpec>>,
    /// Hits per breakpoint, for `hitCondition`. Reset whenever the client
    /// re-sends breakpoints, so editing them restarts the count.
    hits: HashMap<(super::paths::NormPath, u32), u32>,
    /// Memo of the last chunk name seen, since consecutive line events almost
    /// always share one — this keeps normalization off the hot path.
    last_src: Option<(String, Option<super::paths::NormPath>)>,
    step: StepState,
    /// The stop id `park_and_yield` just allocated, for the engine to pair with
    /// the coroutine it gets back. Taken, not read: it belongs to exactly one
    /// suspension.
    parked_id: Option<super::state::StopId>,
    /// Logpoint lines emitted in the current dispatch, against
    /// [`MAX_LOGPOINTS_PER_DISPATCH`]. Per dispatch rather than per second, so
    /// the bound is on the thing a client actually experiences: one command
    /// producing an unreadable wall of console output.
    logpoints: u32,
    /// The request channel, claimed from `DebugState` at startup.
    vm_rx: Option<std::sync::mpsc::Receiver<VmRequest>>,

    /// Instructions charged against the budget in the current dispatch,
    /// accumulated in whole `DebugState::instruction_step` units.
    ///
    /// Reset per dispatch by [`begin_budget`] and *never* mid-dispatch. Zeroing
    /// it when the limit trips would hand `while true do pcall(f) end` a fresh
    /// budget on every iteration, which is no limit at all.
    instr_used: u64,
}

/// What a pending step is waiting for. Depths are measured from the real VM
/// stack, never counted, for the reasons in [`stack_depth`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StepState {
    #[default]
    None,
    /// Stop at the next line, whatever the depth.
    In,
    /// Stop at the next line at or above this depth.
    Over(u16),
    /// Stop at the next line strictly above this depth.
    Out(u16),
}

impl HookLocal {
    pub fn new() -> Self {
        Self {
            gen_seen: u64::MAX,
            mode: TraceMode::Off,
            wants: 0,
            intern: HashMap::new(),
            depth: 0,
            base_depth: None,
            t0: Instant::now(),
            lines: 0,
            calls: 0,
            max_depth: 0,
            bps: HashMap::new(),
            hits: HashMap::new(),
            last_src: None,
            step: StepState::None,
            parked_id: None,
            logpoints: 0,
            vm_rx: None,
            instr_used: 0,
        }
    }

    /// Claim the VM request channel. Called once, from the Lua thread.
    pub fn attach_channel(&mut self, rx: std::sync::mpsc::Receiver<VmRequest>) {
        self.vm_rx = Some(rx);
    }

    fn intern(&mut self, s: &str) -> Arc<str> {
        if let Some(a) = self.intern.get(s) {
            return a.clone();
        }
        let a: Arc<str> = Arc::from(s);
        self.intern.insert(s.to_string(), a.clone());
        a
    }

    /// Re-read the cold config if it changed since we last looked.
    fn refresh(&mut self, st: &SharedDebugState) {
        let gen = st.generation.load(Ordering::Acquire);
        if gen == self.gen_seen {
            return;
        }
        self.gen_seen = gen;
        self.mode = st.trace_config().mode;
        self.wants = st.want.load(Ordering::Relaxed);
        self.bps = st.breakpoints.lock_recover().by_file.clone();
        // Editing breakpoints restarts hit counts; a stale count would make a
        // `hitCondition` fire at a surprising moment.
        self.hits.clear();
        // Chunk-name memo may map to a file whose breakpoints just changed.
        self.last_src = None;
    }

    /// Normalized path for a chunk name, memoized against the previous lookup.
    fn key_for(&mut self, chunk: &str) -> Option<super::paths::NormPath> {
        if let Some((cached, key)) = &self.last_src {
            if cached == chunk {
                return key.clone();
            }
        }
        let key = super::paths::chunk_key(chunk);
        self.last_src = Some((chunk.to_string(), key.clone()));
        key
    }

    fn reset_counters(&mut self) {
        self.t0 = Instant::now();
        self.lines = 0;
        self.calls = 0;
        self.depth = 0;
        self.base_depth = None;
        self.max_depth = 0;
        self.logpoints = 0;
    }
}

impl Default for HookLocal {
    fn default() -> Self {
        Self::new()
    }
}

/// True stack depth, measured with `lua_getstack`.
///
/// Bounded so a runaway recursion cannot make the hook itself pathological.
fn stack_depth(lua: &Lua) -> u16 {
    let mut n = 0u16;
    // mlua 0.11 hands the frame to a callback rather than returning it, so the
    // borrow cannot outlive the frame it describes. Nothing is needed from it
    // here — only whether the level exists.
    while n < 256 && lua.inspect_stack(n as usize, |_| ()).is_some() {
        n += 1;
    }
    n
}

/// Whether a pending step's condition is met at the current line.
fn step_satisfied(lua: &Lua, step: StepState) -> bool {
    match step {
        StepState::None => false,
        StepState::In => true,
        // Depth is re-measured rather than counted: a Call/Ret counter drifts
        // across tail calls, and step-over would then never fire again.
        StepState::Over(d) => stack_depth(lua) <= d,
        StepState::Out(d) => stack_depth(lua) < d,
    }
}

/// What a breakpoint on the current line wants to happen.
#[derive(Debug, PartialEq)]
enum BpOutcome {
    /// No breakpoint here, or its gates said no.
    Pass,
    Stop,
    /// A logpoint: report this and keep running.
    Log(String),
}

/// What the breakpoint on the current line, if any, wants to happen.
///
/// Resolving the chunk name is the expensive part, so this bails on the cheap
/// checks first. Conditions are only compiled and run once a line has actually
/// matched, which keeps them off the hot path entirely.
fn at_breakpoint(
    lua: &Lua,
    st: &SharedDebugState,
    hl: &mut HookLocal,
    dbg: &LuaDebug,
) -> BpOutcome {
    if hl.bps.is_empty() {
        return BpOutcome::Pass;
    }
    let source = dbg.source();
    if source.what == "C" {
        return BpOutcome::Pass;
    }
    // mlua 0.11 returns `Option<usize>`: `None` is "no line information",
    // which is the same answer as the old `<= 0` and means no breakpoint can
    // apply here.
    let Some(line) = dbg.current_line().filter(|l| *l > 0) else {
        return BpOutcome::Pass;
    };
    let Some(chunk) = source.source.as_deref() else { return BpOutcome::Pass };
    let Some(key) = hl.key_for(chunk) else { return BpOutcome::Pass };
    let Some(spec) = hl.bps.get(&key).and_then(|m| m.get(&(line as u32))).cloned() else {
        return BpOutcome::Pass;
    };
    if spec.is_plain() {
        return BpOutcome::Stop;
    }

    // `hitCondition` counts *reached* lines, not stops, so it composes with a
    // condition the way an editor's UI implies.
    let counter = hl.hits.entry((key, line as u32)).or_insert(0);
    *counter += 1;
    let hits = *counter;
    if let Some(needed) = spec.hit_condition {
        if hits < needed {
            return BpOutcome::Pass;
        }
    }

    // A logpoint passes the same gates and then reports instead of stopping.
    let fire = |lua: &Lua| match spec.log_message.as_deref() {
        Some(msg) => BpOutcome::Log(render_log_message(lua, msg)),
        None => BpOutcome::Stop,
    };

    match spec.condition.as_deref() {
        None => fire(lua),
        Some(expr) => match super::introspect::eval_condition(lua, 0, expr) {
            Condition::Met => fire(lua),
            Condition::NotMet => BpOutcome::Pass,
            // Stop and say why. Never stopping would be indistinguishable from
            // a broken breakpoint; always stopping silently is just as opaque.
            Condition::Failed(err) => {
                st.emit(DebugEventMsg::problem(format!(
                    "Breakpoint condition failed at {}:{} — {}\r\n  condition: {}\r\n",
                    super::paths::short(chunk),
                    line,
                    err,
                    expr
                )));
                fire(lua)
            }
        },
    }
}

/// The stop id of the suspension that just happened, if one did.
pub fn take_parked_id(hl: &Rc<RefCell<HookLocal>>) -> Option<super::state::StopId> {
    hl.try_borrow_mut().ok().and_then(|mut hl| hl.parked_id.take())
}

/// Logpoint lines one dispatch may emit before the rest are counted instead.
///
/// A logpoint on a hot line is easy to write by accident — one inside a loop is
/// thousands of console events for a single command, which drowns the client and
/// the thing you were looking for with it. Same bargain as `compute_log`'s
/// `MAX_LOG_LINES`: report a bounded number, then say how many were dropped.
const MAX_LOGPOINTS_PER_DISPATCH: u32 = 200;

/// Send a logpoint's line, or count it against the cap.
fn emit_logpoint(st: &SharedDebugState, hl: &mut HookLocal, text: String) {
    hl.logpoints = hl.logpoints.saturating_add(1);
    match hl.logpoints.cmp(&MAX_LOGPOINTS_PER_DISPATCH) {
        std::cmp::Ordering::Less => st.emit(DebugEventMsg::output(text)),
        // The one that hits the cap says so, so the silence afterwards is
        // explained rather than looking like the logpoint stopped working.
        // Marked as a problem, because losing lines is one.
        std::cmp::Ordering::Equal => st.emit(DebugEventMsg::problem(format!(
            "{text}\r\n[logpoint limit of {MAX_LOGPOINTS_PER_DISPATCH} reached for this \
             dispatch; further lines suppressed]"
        ))),
        std::cmp::Ordering::Greater => {}
    }
}

/// Substitute every `{expr}` in a logpoint message with what it evaluates to in
/// the current frame.
///
/// Braces are the DAP's own syntax, so a message written for VS Code works here
/// unchanged. An expression that fails renders as `expr=<error>` in place rather
/// than failing the whole line: a logpoint that quietly stopped reporting
/// because one field went nil would be worse than useless on the line it is
/// watching.
fn render_log_message(lua: &Lua, template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            // An unmatched brace is a literal, not an error.
            out.push('{');
            rest = after;
            continue;
        };
        let expr = after[..close].trim();
        if expr.is_empty() {
            out.push_str("{}");
        } else {
            match super::introspect::evaluate(lua, 0, expr) {
                Ok(v) => out.push_str(&v.value),
                Err(e) => {
                    out.push_str(expr);
                    out.push_str("=<");
                    out.push_str(&e);
                    out.push('>');
                }
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Build a DAP stack trace by walking the live VM stack.
///
/// Only callable from the Lua thread. C frames are reported without a source so
/// the client shows them but cannot try to open a file for them.
fn build_stack(lua: &Lua, levels: usize) -> Vec<Frame> {
    let mut frames = Vec::new();
    for level in 0..levels.min(256) {
        // The frame is only valid inside the callback (mlua 0.11), so build the
        // owned `Frame` in there and hand it back out.
        let frame = lua.inspect_stack(level, |d| {
            let source = d.source();
            let is_c = source.what == "C";
            let chunk = source.source.as_deref().unwrap_or("").to_string();
            // A function invoked through `pcall` — which is how the mudlib calls
            // every command (`commands.lua:205`) — has no name the VM can report,
            // so fall back to where it was defined rather than a bare "?".
            let name = d.names().name.map(|n| n.to_string()).unwrap_or_else(|| {
                if source.what == "main" {
                    "main chunk".to_string()
                } else if let Some(def) = source.line_defined {
                    format!("{}:{}", super::paths::short(&chunk), def)
                } else {
                    "?".to_string()
                }
            });

            Frame {
                id: level as i64,
                name,
                path: if is_c { None } else { super::paths::display_path(&chunk) },
                line: d.current_line().unwrap_or(0) as u32,
            }
        });

        match frame {
            Some(f) => frames.push(f),
            None => break,
        }
    }
    frames
}

/// Borrow the DAP request channel off the Lua thread's hook state.
///
/// Taken rather than borrowed in place: the caller services requests that call
/// back into the hook state, and holding a `RefCell` borrow across that would
/// panic. Put it back with [`put_channel`].
#[cfg(not(feature = "luajit"))]
pub fn take_channel(
    hl: &Rc<RefCell<HookLocal>>,
) -> Option<std::sync::mpsc::Receiver<VmRequest>> {
    hl.try_borrow_mut().ok()?.vm_rx.take()
}

#[cfg(not(feature = "luajit"))]
pub fn put_channel(hl: &Rc<RefCell<HookLocal>>, rx: std::sync::mpsc::Receiver<VmRequest>) {
    if let Ok(mut h) = hl.try_borrow_mut() {
        h.vm_rx = Some(rx);
    }
}

/// Arm the step a resume asked for, against the depth the stop recorded.
#[cfg(not(feature = "luajit"))]
pub fn arm_step(hl: &Rc<RefCell<HookLocal>>, kind: ResumeKind, depth: u16) {
    if let Ok(mut h) = hl.try_borrow_mut() {
        h.step = match kind {
            ResumeKind::Continue => StepState::None,
            ResumeKind::StepIn => StepState::In,
            ResumeKind::Next => StepState::Over(depth),
            ResumeKind::StepOut => StepState::Out(depth),
        };
    }
}

/// Suspend *this dispatch* and let the rest of the game carry on.
///
/// The Lua 5.3+ path. Returning `VmState::Yield` from a hook suspends the
/// coroutine the dispatch is running on; `engine.rs` parks it and goes back to
/// serving other players, so a breakpoint costs one player their turn instead
/// of costing everyone the server.
///
/// The frames are captured here, before yielding, because this is the last
/// moment they are the *current* stack. Everything the client asks for
/// afterwards — scopes, variables, evaluate — is answered from that capture on
/// whatever thread happens to be running by then. That works because the
/// evaluator was already snapshot-based: `frame_env` has always built an eager
/// copy rather than a live proxy, for its own reasons.
#[cfg(not(feature = "luajit"))]
fn park_and_yield(lua: &Lua, st: &SharedDebugState, hl: &mut HookLocal, reason: StopReason) {
    // Frames first: `build_stack` walks the live stack and must run here.
    let frames = build_stack(lua, 64);
    let capture = super::introspect::capture(lua, 64);

    let session = super::current_dispatch_session().unwrap_or_default();
    let id = st.next_stop_id();
    st.park(
        id,
        super::state::ParkedStop {
            session: session.clone(),
            what: super::current_dispatch_label(),
            reason,
            frames,
            capture,
            depth: stack_depth(lua),
            since: Instant::now(),
        },
    );
    hl.parked_id = Some(id);

    // Clear any pending step so the resume decides afresh; the engine sets the
    // new one when the client says which kind of resume it wants.
    hl.step = StepState::None;

    // `stopped` is deliberately *not* set: the world is not stopped, this one
    // dispatch is. Anything asking "is the game frozen" must keep getting the
    // right answer while an admin debugs a server people are playing on.
    st.emit(DebugEventMsg::Stopped { stop: id, reason, world: false });
}

/// Whether the running Lua code can actually be suspended right now.
///
/// "Is this a coroutine" is *not* the question, and answering that one instead
/// is the bug this exists to avoid. A breakpoint inside a `gsub` replacement
/// function, a `table.sort` comparator or an `__index` metamethod is on the
/// dispatch coroutine and still cannot yield, because a C frame sits between it
/// and the resume. `lua_isyieldable` is the complete answer: it is false on the
/// main thread *and* across any such frame.
///
/// Getting it wrong is quiet rather than loud, which is why it is worth an
/// `unsafe` call. mlua ignores a `VmState::Yield` it cannot honour — see
/// `process_status` in its `state/raw.rs`; execution simply continues — so the
/// stop would leave `DebugState::parked` describing a suspension that never
/// happened and `stopped` true for the rest of the process, with every later
/// debug request refused on those grounds.
#[cfg(not(feature = "luajit"))]
fn can_yield_here(lua: &Lua) -> bool {
    // `lua_topointer` of a thread value is its `lua_State *`, and inside a hook
    // callback `current_thread` is whichever thread ran the hook.
    let state = lua.current_thread().to_pointer() as *mut mlua::lua_State;
    // SAFETY: `state` is the running thread, alive for the duration of this
    // callback, and `lua_isyieldable` only reads a counter on it.
    unsafe { mlua::ffi::lua_isyieldable(state) != 0 }
}

/// Block the Lua thread, and with it the whole game, until the client resumes.
///
/// Always the LuaJIT path — `VmState::Yield` raises there, so there is nowhere
/// to suspend to and blocking is the only way to hold execution. On the
/// yielding runtimes it is the fallback for code that is *not* on a coroutine:
/// timers, connects, GMCP, hot reloads. Yielding from a hook on the main thread
/// raises, so the choice there is block or do not stop at all.
///
/// Every VM-touching DAP request is serviced from right here, because this
/// thread is the only one that may touch the VM and it is not going anywhere
/// until we return.
///
/// The loop always terminates: `recv_timeout` bounds the wait, a disconnected
/// channel breaks out, and the deadline is absolute rather than per-iteration so
/// servicing requests cannot extend it indefinitely.
fn enter_pause(lua: &Lua, st: &SharedDebugState, hl: &mut HookLocal, reason: StopReason) {
    let Some(rx) = hl.vm_rx.take() else {
        tracing::warn!("debugger: stop requested but no request channel; continuing");
        return;
    };

    // The world really is frozen here, which is what `stopped` now means.
    st.stopped.store(true, Ordering::Release);
    st.emit(DebugEventMsg::Stopped {
        stop: super::state::WORLD_STOP,
        reason,
        world: true,
    });

    // Game time does not pass while the world is frozen. Everything the mudlib
    // knows about time is `os_time()` — regeneration settles against it,
    // cooldowns and effects expire against it — and the clock does not care
    // that the VM is blocked in this function. Without this, a minute spent
    // reading a stack trace heals the monster you are fighting by twenty hit
    // points, and combat looks endless for no visible reason.
    let frozen_at = Instant::now();

    let deadline = Instant::now() + st.auto_continue;
    let mut next_step = StepState::None;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            tracing::warn!(
                "debugger: no client response after {:?} — auto-continuing so the game is not wedged",
                st.auto_continue
            );
            break;
        }
        match rx.recv_timeout(remaining) {
            // The stop id is ignored on this path: the world is frozen, so
            // there is exactly one stop and it is this one.
            Ok(VmRequest::StackTrace { levels, reply, .. }) => {
                let _ = reply.send(build_stack(lua, levels));
            }
            Ok(VmRequest::Scopes { frame, reply, .. }) => {
                let _ = reply.send(super::introspect::scopes(lua, frame));
            }
            Ok(VmRequest::Variables { var_ref, reply }) => {
                let _ = reply.send(super::introspect::variables(lua, var_ref));
            }
            Ok(VmRequest::Evaluate { frame, expr, reply, .. }) => {
                let _ = reply.send(super::introspect::evaluate(lua, frame, &expr));
            }
            Ok(VmRequest::Resume { kind, .. }) => {
                next_step = match kind {
                    ResumeKind::Continue => StepState::None,
                    ResumeKind::StepIn => StepState::In,
                    ResumeKind::Next => StepState::Over(stack_depth(lua)),
                    ResumeKind::StepOut => StepState::Out(stack_depth(lua)),
                };
                break;
            }
            Ok(VmRequest::Detach) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                tracing::warn!("debugger: client channel closed while stopped — continuing");
                break;
            }
        }
    }

    // Handles must not outlive the stop that created them — otherwise the
    // variables pane would show values from a frame that no longer exists.
    super::introspect::reset(lua);

    // Banked before the VM runs again, so the first `os_time()` after a resume
    // already excludes this stop rather than seeing the clock jump.
    st.add_paused(frozen_at.elapsed());

    hl.step = next_step;
    hl.vm_rx = Some(rx);
    st.stopped.store(false, Ordering::Release);
    st.emit(DebugEventMsg::Continued {
        stop: super::state::WORLD_STOP,
        world: true,
    });
}

/// The hook callback. Runs on the Lua thread for every call, return, and line.
pub fn on_event(
    lua: &Lua,
    // Borrowed, not owned: mlua 0.11 lends the frame for the duration of the
    // callback rather than handing it over.
    dbg: &LuaDebug,
    st: &SharedDebugState,
    hl: &Rc<RefCell<HookLocal>>,
) -> LuaResult<VmState> {
    // Fast path: a couple of relaxed atomic loads and a thread-local Cell read.
    // A stale read here just means one extra or one missing traced line.
    if !st.armed.load(Ordering::Relaxed) {
        return Ok(VmState::Continue);
    }
    let wants = st.want.load(Ordering::Relaxed);
    if wants == 0 {
        return Ok(VmState::Continue);
    }

    let event = dbg.event();

    // ── the instruction budget ───────────────────────────────────────────
    // Checked before everything else and independent of whether anyone is
    // tracing or debugging this dispatch. The whole game runs on one thread
    // with no preemption, so this error is the only thing standing between
    // `while true do end` in a room file and a server that needs SIGKILL.
    //
    // A debug client being attached suspends it: stepping through a breakpoint
    // is meant to take as long as it takes, and killing the dispatch out from
    // under the editor would look like a crash.
    if wants & want::BUDGET != 0 && st.clients.load(Ordering::Relaxed) == 0 {
        if let Ok(mut hl) = hl.try_borrow_mut() {
            if event == DebugEvent::Count {
                hl.instr_used = hl.instr_used.saturating_add(st.instruction_step as u64);
            }
            if hl.instr_used > st.instruction_limit {
                // Raise on any delivered event once the budget is gone, not
                // only the count that spent it — while tracing is on, a line
                // event may come first.
                //
                // KNOWN GAP. `pcall` catches this error, and Lua 5.1 has no
                // uncatchable one, so
                //
                //     while true do pcall(function() while true do end end) end
                //
                // survives: every raise lands inside the inner loop, the
                // counter restarts there, and the next count event is a full
                // step later — inside the inner loop again. The position is
                // fixed rather than random, so the outer loop is never reached.
                // Widening the mask does not help; see `HookShape::for_state`.
                // Deliberately written code can still hang the game thread.
                // `docs/src/lua-api/sandboxing.md` says so.
                return Err(LuaError::RuntimeError(format!(
                    "instruction limit exceeded ({} instructions in one dispatch)",
                    st.instruction_limit
                )));
            }
        }
    }
    // Count events carry no source position, so nothing below applies to them.
    if event == DebugEvent::Count {
        return Ok(VmState::Continue);
    }

    // Tracing is scoped to opted-in sessions; debugging deliberately is not.
    // A breakpoint that only fired for sessions you had separately opted in
    // would look exactly like a broken debugger.
    let debugging = st.clients.load(Ordering::Relaxed) > 0;
    let tracing = super::dispatch_is_traced();
    if !debugging && !tracing {
        return Ok(VmState::Continue);
    }

    let Ok(mut hl) = hl.try_borrow_mut() else {
        // Re-entered while we already hold the state; nothing safe to do.
        return Ok(VmState::Continue);
    };
    hl.refresh(st);

    // `TailCall` means opposite things on the two runtimes, and getting it
    // wrong is silent: a tail-recursive function reports either one call or
    // hundreds of unmatched returns.
    //
    //   5.1 / LuaJIT  LUA_HOOKTAILRET — the frame is *leaving*
    //   5.4+          LUA_HOOKTAILCALL — the frame is being *entered*
    //
    // Depth is measured from the real VM stack either way, so only the counter
    // and the trace glyph depend on this.
    #[cfg(feature = "luajit")]
    let tail_is_a_call = false;
    #[cfg(not(feature = "luajit"))]
    let tail_is_a_call = true;

    let is_call = matches!(event, DebugEvent::Call)
        || (tail_is_a_call && matches!(event, DebugEvent::TailCall));
    let is_ret = matches!(event, DebugEvent::Ret)
        || (!tail_is_a_call && matches!(event, DebugEvent::TailCall));

    let kind = if is_call {
        hl.calls = hl.calls.saturating_add(1);
        // One stack walk per call — calls are far rarer than lines, and this
        // is the only way to get a depth that survives tail calls.
        let abs = stack_depth(lua);
        let base = *hl.base_depth.get_or_insert(abs.saturating_sub(1));
        hl.depth = abs.saturating_sub(base);
        hl.max_depth = hl.max_depth.max(hl.depth);
        TraceKind::Call
    } else if is_ret {
        hl.depth = hl.depth.saturating_sub(1);
        TraceKind::Ret
    } else if matches!(event, DebugEvent::Line) {
        hl.lines = hl.lines.saturating_add(1);
        TraceKind::Line
    } else {
        return Ok(VmState::Continue);
    };

    // ── breakpoints, stepping, and explicit pause ────────────────────────
    // Only line events can stop: a call or return event's reported position is
    // the *caller's*, which would show the wrong line in the editor.
    if kind == TraceKind::Line && debugging {
        let stop = if st.pause_req.swap(false, Ordering::AcqRel) {
            Some(StopReason::Pause)
        } else if hl.step != StepState::None && step_satisfied(lua, hl.step) {
            Some(StopReason::Step)
        } else if st.bp_count.load(Ordering::Relaxed) > 0 {
            match at_breakpoint(lua, st, &mut hl, dbg) {
                BpOutcome::Stop => Some(StopReason::Breakpoint),
                // A logpoint. Report and carry on — the point of it is that the
                // line it watches is one execution reaches over and over.
                BpOutcome::Log(text) => {
                    emit_logpoint(st, &mut hl, text);
                    None
                }
                BpOutcome::Pass => None,
            }
        } else {
            None
        };

        if let Some(reason) = stop {
            #[cfg(feature = "luajit")]
            enter_pause(lua, st, &mut hl, reason);

            // Suspend only when asked to *and* able to. `stop_the_world` is the
            // policy — freeze like every other debugger, or hold one dispatch
            // and let the game carry on — and `can_yield_here` is whether Lua
            // can honour it here at all.
            #[cfg(not(feature = "luajit"))]
            if !st.freezes() && can_yield_here(lua) {
                park_and_yield(lua, st, &mut hl, reason);
                // The trace record for this event is skipped: the dispatch is
                // suspended here and the engine takes over.
                return Ok(VmState::Yield);
            } else {
                // Either the policy says freeze, or there is nowhere to suspend
                // to — the main thread, or a C frame in the way.
                enter_pause(lua, st, &mut hl, reason);
            }
        }
    }

    // Counters are all `Timing` mode asks for; records cost a `lua_getinfo`.
    if !tracing || wants & want::TRACE == 0 {
        return Ok(VmState::Continue);
    }
    if kind == TraceKind::Line && hl.mode != TraceMode::Lines {
        return Ok(VmState::Continue);
    }

    let source = dbg.source();
    // C functions (efuns, string.*, ...) have no chunk or line — reporting them
    // as `=[C]:0` is just noise, so they get a bare marker and line 0.
    let is_c = source.what == "C";
    let src = if is_c {
        hl.intern("[C]")
    } else {
        hl.intern(source.source.as_deref().unwrap_or("?"))
    };
    let line = if is_c { 0 } else { dbg.current_line().unwrap_or(0) as u32 };
    let name = dbg.names().name.map(|n| hl.intern(&n));
    let micros = hl.t0.elapsed().as_micros().min(u32::MAX as u128) as u32;
    let depth = hl.depth;

    trace::push_record(TraceRecord { kind, depth, src, line, name, micros });

    Ok(VmState::Continue)
}

/// Hand the next dispatch a full instruction budget.
///
/// Called by the engine before *every* command, so the limit is per dispatch
/// rather than per process. Separate from [`begin_dispatch`], which only wraps
/// player input: a timer tick or a GMCP message needs the budget just as much,
/// but has no timing record to reset.
pub fn begin_budget(hl: &Rc<RefCell<HookLocal>>) {
    if let Ok(mut hl) = hl.try_borrow_mut() {
        hl.instr_used = 0;
    }
}

/// Start timing an input dispatch.
pub fn begin_dispatch(hl: &Rc<RefCell<HookLocal>>) {
    if let Ok(mut hl) = hl.try_borrow_mut() {
        hl.reset_counters();
    }
}

/// Finish an input dispatch and record its timing.
pub fn end_dispatch(hl: &Rc<RefCell<HookLocal>>, session: &str, verb: &str) {
    let Ok(hl) = hl.try_borrow() else { return };
    trace::push_timing(CommandTiming {
        session: session.to_string(),
        verb: verb.to_string(),
        micros: hl.t0.elapsed().as_micros().min(u64::MAX as u128) as u64,
        lines: hl.lines,
        calls: hl.calls,
        max_depth: hl.max_depth,
    });
}
