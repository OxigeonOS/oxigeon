//! The control block shared between the Lua thread and the outside world.
//!
//! Everything the hook reads on its hot path is an atomic. The cold state behind
//! the mutex is copied into a Lua-thread-private cache and only re-read when
//! [`DebugState::generation`] changes, so a traced line never takes a lock.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Bits in [`DebugState::want`] — what the hook callback should actually do.
///
/// The installed `HookTriggers` mask is *constant* while armed; these gate the
/// work inside the callback. A varying mask would require calling `set_hook`
/// from inside the hook, which mlua refuses (`hook_proc` bails when the callback
/// `Rc` is already borrowed).
pub mod want {
    /// Append per-event records to the trace ring.
    pub const TRACE: u8 = 1 << 0;
    /// Maintain per-dispatch line/call counters.
    pub const TIMING: u8 = 1 << 1;
    /// Check breakpoints on line events. (M3)
    pub const BREAK: u8 = 1 << 2;
    /// A step is pending. (M3)
    pub const STEP: u8 = 1 << 3;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TraceMode {
    #[default]
    Off,
    /// Counters only — no per-event records. The cheapest useful mode.
    Timing,
    /// Function entry and exit.
    Calls,
    /// Every executed line.
    Lines,
}

impl TraceMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "time" | "timing" => Some(Self::Timing),
            "call" | "calls" | "on" => Some(Self::Calls),
            "line" | "lines" => Some(Self::Lines),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Timing => "time",
            Self::Calls => "calls",
            Self::Lines => "lines",
        }
    }

    /// Which `want` bits this mode needs.
    fn wants(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Timing => want::TIMING,
            Self::Calls | Self::Lines => want::TIMING | want::TRACE,
        }
    }
}

/// Which sessions are being traced.
#[derive(Default, Clone)]
pub struct TraceConfig {
    pub mode: TraceMode,
    pub all_sessions: bool,
    pub sessions: HashSet<String>,
}

impl TraceConfig {
    /// Whether events dispatched for `session_id` should be recorded.
    ///
    /// A `None` session means an engine-internal dispatch (a timer, say) with no
    /// originating player; those are only traced when tracing everything.
    pub fn covers(&self, session_id: Option<&str>) -> bool {
        if self.mode == TraceMode::Off {
            return false;
        }
        if self.all_sessions {
            return true;
        }
        session_id.is_some_and(|s| self.sessions.contains(s))
    }
}

/// Why the VM stopped, as a DAP `stopped` event reason.
#[derive(Clone, Copy, Debug)]
pub enum StopReason {
    Breakpoint,
    Step,
    Pause,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Breakpoint => "breakpoint",
            Self::Step => "step",
            Self::Pause => "pause",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeKind {
    Continue,
    Next,
    StepIn,
    StepOut,
}

/// One frame of a DAP `stackTrace` response.
#[derive(Clone, Debug)]
pub struct Frame {
    pub id: i64,
    pub name: String,
    /// Absolute path in the casing the client can open, or `None` for C frames
    /// and chunks with no backing file.
    pub path: Option<String>,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct DapScope {
    pub name: String,
    pub var_ref: i64,
    pub expensive: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DapVariable {
    pub name: String,
    pub value: String,
    pub ty: String,
    /// Non-zero if the value has children the client can expand.
    pub var_ref: i64,
}

/// A request that can only be serviced by the Lua thread, and only while it is
/// stopped inside the hook.
pub enum VmRequest {
    StackTrace {
        levels: usize,
        reply: tokio::sync::oneshot::Sender<Vec<Frame>>,
    },
    Scopes {
        frame: i64,
        reply: tokio::sync::oneshot::Sender<Vec<DapScope>>,
    },
    Variables {
        var_ref: i64,
        reply: tokio::sync::oneshot::Sender<Vec<DapVariable>>,
    },
    Evaluate {
        frame: i64,
        expr: String,
        reply: tokio::sync::oneshot::Sender<Result<DapVariable, String>>,
    },
    Resume(ResumeKind),
    Detach,
}

/// A notification from the Lua thread to the DAP client.
#[derive(Debug)]
pub enum DebugEventMsg {
    Stopped(StopReason),
    Continued,
    Output(String),
}

/// What must hold before a breakpoint actually stops the VM.
#[derive(Clone, Debug, Default)]
pub struct BreakpointSpec {
    /// A Lua expression evaluated in the paused frame. Stop only if truthy.
    pub condition: Option<String>,
    /// Ignore this many hits before stopping.
    pub hit_condition: Option<u32>,
}

impl BreakpointSpec {
    pub fn is_plain(&self) -> bool {
        self.condition.is_none() && self.hit_condition.is_none()
    }
}

#[derive(Default)]
pub struct BreakpointTable {
    /// normalized path -> line -> spec
    pub by_file:
        std::collections::HashMap<super::paths::NormPath, std::collections::HashMap<u32, BreakpointSpec>>,
    pub next_id: i64,
}

pub struct DebugState {
    // ── hot: read on every hook event, never locked ──────────────────────
    /// Master gate. False means the hook is not installed at all.
    pub armed: AtomicBool,
    /// Bitmask of [`want`] flags.
    pub want: AtomicU8,
    /// Bumped on every config change; the Lua thread re-reads `cfg` when it moves.
    pub generation: AtomicU64,
    /// Total breakpoints across all files — lets a line event reject in one load.
    pub bp_count: AtomicU64,
    /// Attached DAP clients. Non-zero means breakpoints are live.
    pub clients: AtomicU64,
    /// True while the Lua thread is blocked in the pause loop. The DAP task
    /// checks this before queuing a VM-touching request, so such a request can
    /// never sit unanswered in the channel until the next breakpoint.
    pub stopped: AtomicBool,
    /// A `pause` request from the client, consumed by the next line event.
    pub pause_req: AtomicBool,

    // ── cold: locked only when something changes ─────────────────────────
    cfg: Mutex<TraceConfig>,
    pub breakpoints: Mutex<BreakpointTable>,

    // ── channels ─────────────────────────────────────────────────────────
    /// Lua thread -> DAP task.
    pub evt_tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<DebugEventMsg>>>,
    /// DAP task -> Lua thread. `std::sync::mpsc` rather than tokio's, because
    /// only its `recv_timeout` gives the auto-continue safety valve.
    pub vm_tx: Mutex<Option<std::sync::mpsc::Sender<VmRequest>>>,
    /// Receiving end, taken exactly once by the Lua thread at startup.
    pub vm_rx: Mutex<Option<std::sync::mpsc::Receiver<VmRequest>>>,

    pub auto_continue: std::time::Duration,
    pub trace_capacity: usize,
    pub timing_capacity: usize,
    /// Load the Lua `debug` stdlib so variables and `evaluate` work.
    ///
    /// This requires `unsafe_new_with`, so it is only ever true when the debug
    /// adapter is explicitly enabled. The table is hidden from `_G` immediately.
    pub debug_library: bool,
}

pub type SharedDebugState = Arc<DebugState>;

impl DebugState {
    pub fn new(trace_capacity: usize, timing_capacity: usize) -> Self {
        let (vm_tx, vm_rx) = std::sync::mpsc::channel();
        Self {
            armed: AtomicBool::new(false),
            want: AtomicU8::new(0),
            generation: AtomicU64::new(0),
            bp_count: AtomicU64::new(0),
            clients: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            pause_req: AtomicBool::new(false),
            cfg: Mutex::new(TraceConfig::default()),
            breakpoints: Mutex::new(BreakpointTable::default()),
            evt_tx: Mutex::new(None),
            vm_tx: Mutex::new(Some(vm_tx)),
            vm_rx: Mutex::new(Some(vm_rx)),
            auto_continue: std::time::Duration::from_secs(300),
            trace_capacity,
            timing_capacity,
            debug_library: false,
        }
    }

    pub fn shared(trace_capacity: usize, timing_capacity: usize) -> SharedDebugState {
        Arc::new(Self::new(trace_capacity, timing_capacity))
    }

    pub fn from_config(cfg: &crate::config::DebugServerConfig) -> SharedDebugState {
        let mut st = Self::new(cfg.trace_capacity, cfg.timing_capacity);
        st.debug_library = cfg.enabled;
        st.auto_continue = if cfg.auto_continue_secs == 0 {
            // "Never" still needs a finite value; a day is effectively never and
            // keeps the pause loop's timeout arithmetic total.
            std::time::Duration::from_secs(86_400)
        } else {
            std::time::Duration::from_secs(cfg.auto_continue_secs)
        };
        Arc::new(st)
    }

    /// Claim the request receiver. Returns `None` on any call after the first —
    /// the Lua thread is its only legitimate owner.
    pub fn take_vm_rx(&self) -> Option<std::sync::mpsc::Receiver<VmRequest>> {
        self.vm_rx.lock().unwrap().take()
    }

    pub fn trace_config(&self) -> TraceConfig {
        self.cfg.lock().unwrap().clone()
    }

    /// Recompute the hot-path atomics from every input that feeds them.
    ///
    /// `generation` is bumped last with `Release` so a reader that observes the
    /// new generation is guaranteed to see the new state behind the mutexes.
    pub fn republish(&self) {
        let mut wants = self.cfg.lock().unwrap().wants();
        // While a client is attached the hook stays installed even with no
        // breakpoints set, so `pause` can interrupt a running VM.
        if self.clients.load(Ordering::Relaxed) > 0 {
            wants |= want::BREAK;
        }
        self.want.store(wants, Ordering::Relaxed);
        self.armed.store(wants != 0, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn set_trace_config(&self, new: TraceConfig) {
        *self.cfg.lock().unwrap() = new;
        self.republish();
    }

    /// Mutate the trace config in place, then republish.
    pub fn update_trace_config(&self, f: impl FnOnce(&mut TraceConfig)) {
        let mut cfg = self.cfg.lock().unwrap().clone();
        f(&mut cfg);
        self.set_trace_config(cfg);
    }

    /// Replace every breakpoint for one file. DAP's `setBreakpoints` is
    /// whole-file replace, not incremental.
    pub fn set_breakpoints(
        &self,
        file: super::paths::NormPath,
        specs: &[(u32, BreakpointSpec)],
    ) -> Vec<i64> {
        let mut bp = self.breakpoints.lock().unwrap();
        let ids = specs
            .iter()
            .map(|_| {
                bp.next_id += 1;
                bp.next_id
            })
            .collect();
        if specs.is_empty() {
            bp.by_file.remove(&file);
        } else {
            bp.by_file.insert(file, specs.iter().cloned().collect());
        }
        let total: usize = bp.by_file.values().map(|m| m.len()).sum();
        drop(bp);
        self.bp_count.store(total as u64, Ordering::Relaxed);
        self.republish();
        ids
    }

    pub fn clear_breakpoints(&self) {
        self.breakpoints.lock().unwrap().by_file.clear();
        self.bp_count.store(0, Ordering::Relaxed);
        self.republish();
    }

    pub fn emit(&self, msg: DebugEventMsg) {
        if let Some(tx) = self.evt_tx.lock().unwrap().as_ref() {
            let _ = tx.send(msg);
        }
    }

    /// Queue a request for the Lua thread. Fails if nothing is listening.
    pub fn send_vm(&self, req: VmRequest) -> bool {
        match self.vm_tx.lock().unwrap().as_ref() {
            Some(tx) => tx.send(req).is_ok(),
            None => false,
        }
    }
}

impl TraceConfig {
    fn wants(&self) -> u8 {
        // An enabled mode with nothing to trace is the same as off.
        if self.all_sessions || !self.sessions.is_empty() {
            self.mode.wants()
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracing_nobody_leaves_the_hook_disarmed() {
        let st = DebugState::new(64, 8);
        st.update_trace_config(|c| c.mode = TraceMode::Lines);
        assert!(
            !st.armed.load(Ordering::Relaxed),
            "a mode with no opted-in sessions must not arm the hook"
        );

        st.update_trace_config(|c| {
            c.sessions.insert("7".into());
        });
        assert!(st.armed.load(Ordering::Relaxed));
        assert_eq!(st.want.load(Ordering::Relaxed), want::TIMING | want::TRACE);
    }

    #[test]
    fn timing_mode_does_not_ask_for_records() {
        let st = DebugState::new(64, 8);
        st.update_trace_config(|c| {
            c.mode = TraceMode::Timing;
            c.all_sessions = true;
        });
        assert_eq!(st.want.load(Ordering::Relaxed), want::TIMING);
    }

    #[test]
    fn turning_off_disarms_and_bumps_generation() {
        let st = DebugState::new(64, 8);
        st.update_trace_config(|c| {
            c.mode = TraceMode::Calls;
            c.all_sessions = true;
        });
        let g = st.generation.load(Ordering::Acquire);

        st.update_trace_config(|c| c.mode = TraceMode::Off);
        assert!(!st.armed.load(Ordering::Relaxed));
        assert!(st.generation.load(Ordering::Acquire) > g);
    }

    #[test]
    fn covers_respects_scope() {
        let mut cfg = TraceConfig { mode: TraceMode::Calls, ..Default::default() };
        cfg.sessions.insert("3".into());
        assert!(cfg.covers(Some("3")));
        assert!(!cfg.covers(Some("4")));
        assert!(!cfg.covers(None), "engine-internal dispatch is not session-scoped");

        cfg.all_sessions = true;
        assert!(cfg.covers(None));
    }
}
