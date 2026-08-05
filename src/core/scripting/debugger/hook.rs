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
    while n < 256 && lua.inspect_stack(n as usize).is_some() {
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

/// Whether the current line should stop the VM.
///
/// Resolving the chunk name is the expensive part, so this bails on the cheap
/// checks first. Conditions are only compiled and run once a line has actually
/// matched, which keeps them off the hot path entirely.
fn at_breakpoint(lua: &Lua, st: &SharedDebugState, hl: &mut HookLocal, dbg: &LuaDebug) -> bool {
    if hl.bps.is_empty() {
        return false;
    }
    let source = dbg.source();
    if source.what == "C" {
        return false;
    }
    let line = dbg.curr_line();
    if line <= 0 {
        return false;
    }
    let Some(chunk) = source.source.as_deref() else { return false };
    let Some(key) = hl.key_for(chunk) else { return false };
    let Some(spec) = hl.bps.get(&key).and_then(|m| m.get(&(line as u32))).cloned() else {
        return false;
    };
    if spec.is_plain() {
        return true;
    }

    // `hitCondition` counts *reached* lines, not stops, so it composes with a
    // condition the way an editor's UI implies.
    let counter = hl.hits.entry((key, line as u32)).or_insert(0);
    *counter += 1;
    let hits = *counter;
    if let Some(needed) = spec.hit_condition {
        if hits < needed {
            return false;
        }
    }

    match spec.condition.as_deref() {
        None => true,
        Some(expr) => match super::introspect::eval_condition(lua, 0, expr) {
            Condition::Met => true,
            Condition::NotMet => false,
            // Stop and say why. Never stopping would be indistinguishable from
            // a broken breakpoint; always stopping silently is just as opaque.
            Condition::Failed(err) => {
                st.emit(DebugEventMsg::Output(format!(
                    "Breakpoint condition failed at {}:{} — {}\r\n  condition: {}\r\n",
                    super::paths::short(chunk),
                    line,
                    err,
                    expr
                )));
                true
            }
        },
    }
}

/// Build a DAP stack trace by walking the live VM stack.
///
/// Only callable from the Lua thread. C frames are reported without a source so
/// the client shows them but cannot try to open a file for them.
fn build_stack(lua: &Lua, levels: usize) -> Vec<Frame> {
    let mut frames = Vec::new();
    for level in 0..levels.min(256) {
        let Some(d) = lua.inspect_stack(level) else { break };
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

        frames.push(Frame {
            id: level as i64,
            name,
            path: if is_c { None } else { super::paths::display_path(&chunk) },
            line: d.curr_line().max(0) as u32,
        });
    }
    frames
}

/// Block the Lua thread, and with it the whole game, until the client resumes.
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

    st.stopped.store(true, Ordering::Release);
    st.emit(DebugEventMsg::Stopped(reason));

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
            Ok(VmRequest::StackTrace { levels, reply }) => {
                let _ = reply.send(build_stack(lua, levels));
            }
            Ok(VmRequest::Scopes { frame, reply }) => {
                let _ = reply.send(super::introspect::scopes(lua, frame));
            }
            Ok(VmRequest::Variables { var_ref, reply }) => {
                let _ = reply.send(super::introspect::variables(lua, var_ref));
            }
            Ok(VmRequest::Evaluate { frame, expr, reply }) => {
                let _ = reply.send(super::introspect::evaluate(lua, frame, &expr));
            }
            Ok(VmRequest::Resume(kind)) => {
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
    st.emit(DebugEventMsg::Continued);
}

/// The hook callback. Runs on the Lua thread for every call, return, and line.
pub fn on_event(
    lua: &Lua,
    dbg: LuaDebug,
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

    let kind = match event {
        DebugEvent::Call => {
            hl.calls = hl.calls.saturating_add(1);
            // One stack walk per call — calls are far rarer than lines, and this
            // is the only way to get a depth that survives tail calls.
            let abs = stack_depth(lua);
            let base = *hl.base_depth.get_or_insert(abs.saturating_sub(1));
            hl.depth = abs.saturating_sub(base);
            hl.max_depth = hl.max_depth.max(hl.depth);
            TraceKind::Call
        }
        // Lua 5.1 reports a tail-call return as TailCall; treat it as a return.
        DebugEvent::Ret | DebugEvent::TailCall => {
            hl.depth = hl.depth.saturating_sub(1);
            TraceKind::Ret
        }
        DebugEvent::Line => {
            hl.lines = hl.lines.saturating_add(1);
            TraceKind::Line
        }
        _ => return Ok(VmState::Continue),
    };

    // ── breakpoints, stepping, and explicit pause ────────────────────────
    // Only line events can stop: a call or return event's reported position is
    // the *caller's*, which would show the wrong line in the editor.
    if kind == TraceKind::Line && debugging {
        let stop = if st.pause_req.swap(false, Ordering::AcqRel) {
            Some(StopReason::Pause)
        } else if hl.step != StepState::None && step_satisfied(lua, hl.step) {
            Some(StopReason::Step)
        } else if st.bp_count.load(Ordering::Relaxed) > 0 && at_breakpoint(lua, st, &mut hl, &dbg) {
            Some(StopReason::Breakpoint)
        } else {
            None
        };

        if let Some(reason) = stop {
            enter_pause(lua, st, &mut hl, reason);
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
    let line = if is_c { 0 } else { dbg.curr_line().max(0) as u32 };
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
