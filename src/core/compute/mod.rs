//! Running long Lua computations off the game thread.
//!
//! The whole game runs on one Lua thread. Anything expensive on it — a
//! pathfind across a large map, generating an area, a simulation pass — freezes
//! every connected player for its duration. This is the escape hatch: hand the
//! work to a pool of worker threads, each with its own LuaJIT VM, and get the
//! answer back later through a mudlib hook.
//!
//! It is deliberately shaped like [`crate::core::auth`], which solves the same
//! problem for Argon2: a bounded queue, a fixed pool, and a round trip through
//! [`LuaCommand`]. Two things differ. The work is arbitrary game code rather
//! than one fixed operation, so it has to be identified and loaded. And its
//! arguments and results must be *copied* between two VMs, because mlua's
//! `Lua` is `!Send` and no Lua value may cross a thread — see [`marshal`].
//!
//! # The contract
//!
//! If [`ComputeBridge::submit`] returns an id, **exactly one** result is
//! delivered for it. If it returns an error, none is. Everything operational —
//! a full queue, a timeout, a cancel, a module that will not load, a job that
//! raises — arrives through the result path, because the mudlib's cleanup is
//! identical for all of them and making a caller handle "the efun told me" and
//! "the hook told me" as separate cases is how that cleanup gets forgotten.
//!
//! # What isolation you actually get
//!
//! A wedged job costs **one worker, not the game**. Rust cannot kill a thread,
//! so with the compiler on (the default) a runaway job burns its worker for
//! the life of the process; the deadline unblocks the *caller* but does not
//! stop the job. Arming `compute.instruction_limit` makes workers recoverable,
//! at the cost of the compiler in that VM. Both are documented and neither is
//! silent: wedged workers are counted and surfaced in `server_info()`.

pub mod marshal;
pub mod vm;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::config::server_config::ComputeConfig;
use crate::core::lock::MutexExt;
use crate::core::scripting::engine::LuaCommand;

pub use marshal::{Limits, LuaData, MarshalError};
pub use vm::Ending;

/// Correlates a submission with its result.
pub type JobId = u64;

/// Why a submission was rejected outright, before any id existed.
///
/// These are all things correct code never does, which is why they are
/// returned to the caller rather than delivered as a result: the mistake is at
/// the call site and the stack is right there.
#[derive(Debug, PartialEq)]
pub enum SubmitError {
    Disabled,
    /// The entry module is not under any configured root.
    BadModule(String),
    /// The arguments could not be copied out of the game VM.
    Args(MarshalError),
    DeadlineTooLong { asked: u64, max: u64 },
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(
                f,
                "compute is disabled; set [compute] enabled = true in server.toml"
            ),
            Self::BadModule(m) => write!(
                f,
                "'{m}' is not under a configured compute root — see [compute] roots"
            ),
            Self::Args(e) => write!(f, "arguments cannot be sent to a compute worker: {e}"),
            Self::DeadlineTooLong { asked, max } => {
                write!(f, "deadline_ms {asked} is above the configured maximum of {max}")
            }
        }
    }
}

struct Job {
    id: JobId,
    module: String,
    func: String,
    args: LuaData,
    deadline: Instant,
}

/// A job the pool has accepted and not yet answered for.
struct Live {
    module: String,
    func: String,
    tag: LuaData,
    submitted: Instant,
    deadline: Instant,
    started: Option<Instant>,
    cancelled: bool,
}

/// Counters behind `server_info().compute`.
///
/// `wedged` is the one an operator should watch: it counts jobs that blew
/// their deadline while still running, which — with the compiler on — means a
/// worker thread that is never coming back.
#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub submitted: u64,
    pub completed: u64,
    pub failed: u64,
    pub timed_out: u64,
    pub refused: u64,
    pub cancelled: u64,
    pub wedged: u64,
}

/// A point-in-time view of the pool, for `server_info()`.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub workers: usize,
    pub queue_depth: usize,
    pub instruction_limit: u64,
    /// Jobs submitted and not yet answered.
    pub in_flight: usize,
    /// Of those, how many are actually running.
    pub running: usize,
    pub stats: Stats,
}

/// Handle to the pool. Cloneable; the workers stop when the engine's command
/// channel closes.
#[derive(Clone)]
pub struct ComputeBridge {
    tx: SyncSender<Job>,
    /// Kept so a refusal or a deadline can be answered from the submitting
    /// thread, not only from a worker.
    cmd_tx: UnboundedSender<LuaCommand>,
    next_id: Arc<AtomicU64>,
    /// Bumped on every reload; workers rebuild their VM when it moves.
    epoch: Arc<AtomicU64>,
    live: Arc<Mutex<HashMap<JobId, Live>>>,
    stats: Arc<Mutex<Stats>>,
    cfg: Arc<ComputeConfig>,
}

impl ComputeBridge {
    /// Start the pool. Returns `None` when compute is disabled, which is what
    /// keeps the efuns unregistered and the feature free when unused.
    pub fn start(
        cfg: ComputeConfig,
        mudlib: PathBuf,
        game: PathBuf,
        cmd_tx: UnboundedSender<LuaCommand>,
    ) -> Option<Self> {
        if !cfg.enabled {
            return None;
        }

        let (tx, rx) = sync_channel::<Job>(cfg.queue_depth.max(1));
        let bridge = Self {
            tx,
            cmd_tx: cmd_tx.clone(),
            next_id: Arc::new(AtomicU64::new(1)),
            epoch: Arc::new(AtomicU64::new(0)),
            live: Arc::new(Mutex::new(HashMap::new())),
            stats: Arc::new(Mutex::new(Stats::default())),
            cfg: Arc::new(cfg),
        };

        let rx = Arc::new(Mutex::new(rx));
        for n in 0..bridge.cfg.workers.max(1) {
            bridge.spawn_worker(n, rx.clone(), mudlib.clone(), game.clone(), cmd_tx.clone());
        }
        bridge.spawn_watchdog();

        tracing::info!(
            "compute: {} worker(s), queue {}, instruction limit {} ({})",
            bridge.cfg.workers,
            bridge.cfg.queue_depth,
            bridge.cfg.instruction_limit,
            if bridge.cfg.instruction_limit > 0 {
                "compiler off, runaway jobs recoverable"
            } else {
                "compiler on, a runaway job burns its worker permanently"
            }
        );

        Some(bridge)
    }

    fn spawn_worker(
        &self,
        n: usize,
        rx: Arc<Mutex<Receiver<Job>>>,
        mudlib: PathBuf,
        game: PathBuf,
        cmd_tx: UnboundedSender<LuaCommand>,
    ) {
        let cfg = self.cfg.clone();
        let epoch = self.epoch.clone();
        let live = self.live.clone();
        let stats = self.stats.clone();

        std::thread::Builder::new()
            .name(format!("oxigeon-compute-{n}"))
            .spawn(move || {
                // The VM is built lazily and rebuilt whenever the epoch moves,
                // so compute costs nothing until it is used and a reload is
                // picked up without any `package.loaded` surgery. Throwing the
                // whole VM away is always safe here precisely because it holds
                // no state anyone is allowed to depend on — which is the
                // property the game VM lacks.
                let mut built: Option<(u64, vm::ComputeVm)> = None;

                loop {
                    let job = match rx.lock_recover().recv() {
                        Ok(job) => job,
                        Err(_) => break, // every sender dropped
                    };

                    // Already answered — timed out in the queue, or cancelled.
                    let still_wanted = {
                        let mut live = live.lock_recover();
                        match live.get_mut(&job.id) {
                            Some(entry) if !entry.cancelled => {
                                entry.started = Some(Instant::now());
                                true
                            }
                            _ => false,
                        }
                    };
                    if !still_wanted {
                        continue;
                    }

                    let current = epoch.load(Ordering::Relaxed);
                    if built.as_ref().is_none_or(|(e, _)| *e != current) {
                        match vm::build(&cfg, &mudlib, &game) {
                            Ok(new) => built = Some((current, new)),
                            Err(e) => {
                                tracing::error!("compute: could not build a VM: {e}");
                                Self::answer(
                                    &cmd_tx, &live, &stats, job.id,
                                    Ending::LoadError,
                                    LuaData::Nil,
                                    Some(format!("compute worker could not start: {e}")),
                                    Vec::new(),
                                );
                                continue;
                            }
                        }
                    }

                    let machine = &built.as_ref().unwrap().1;
                    let out = machine.run(&job.module, &job.func, &job.args, Some(job.deadline));
                    Self::answer(
                        &cmd_tx, &live, &stats, job.id,
                        out.ending, out.value, out.error, out.logs,
                    );
                }
            })
            .expect("failed to spawn a compute worker");
    }

    /// Deliver a result, unless something already answered for this id.
    fn answer(
        cmd_tx: &UnboundedSender<LuaCommand>,
        live: &Arc<Mutex<HashMap<JobId, Live>>>,
        stats: &Arc<Mutex<Stats>>,
        id: JobId,
        ending: Ending,
        value: LuaData,
        error: Option<String>,
        logs: Vec<(String, String)>,
    ) {
        // Claiming the entry is what makes "exactly one result per id" true:
        // whoever removes it first is the one that gets to report.
        let Some(entry) = live.lock_recover().remove(&id) else {
            return;
        };

        {
            let mut s = stats.lock_recover();
            match ending {
                Ending::Ok => s.completed += 1,
                Ending::Timeout => s.timed_out += 1,
                Ending::Cancelled => s.cancelled += 1,
                Ending::Refused => s.refused += 1,
                _ => s.failed += 1,
            }
        }

        let now = Instant::now();
        let started = entry.started.unwrap_or(now);
        let _ = cmd_tx.send(LuaCommand::ComputeResult {
            id,
            kind: ending.as_str(),
            value,
            error,
            tag: entry.tag,
            module: entry.module,
            func: entry.func,
            queued_ms: (started - entry.submitted).as_secs_f64() * 1000.0,
            run_ms: (now - started).as_secs_f64() * 1000.0,
            logs,
        });
    }

    /// Queue a job.
    pub fn submit(
        &self,
        module: String,
        func: String,
        args: LuaData,
        tag: LuaData,
        deadline_ms: Option<u64>,
    ) -> Result<JobId, SubmitError> {
        if !self.module_is_allowed(&module) {
            return Err(SubmitError::BadModule(module));
        }
        let deadline_ms = match deadline_ms {
            None => self.cfg.default_deadline_ms,
            Some(ms) if ms <= self.cfg.max_deadline_ms => ms,
            Some(ms) => {
                return Err(SubmitError::DeadlineTooLong { asked: ms, max: self.cfg.max_deadline_ms })
            }
        };

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        // Measured from submit, not from start of run: it is a latency
        // contract with the caller, and a job that expired while queued is
        // exactly the signal that the queue is the problem.
        let deadline = now + Duration::from_millis(deadline_ms);

        self.live.lock_recover().insert(
            id,
            Live {
                module: module.clone(),
                func: func.clone(),
                tag,
                submitted: now,
                deadline,
                started: None,
                cancelled: false,
            },
        );
        self.stats.lock_recover().submitted += 1;

        let job = Job { id, module, func, args, deadline };
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) = self.tx.try_send(job) {
            // A full queue is transient and operational, so it comes back
            // through the result path like every other operational failure —
            // the id is already allocated and the caller already has cleanup
            // written for a failed result.
            Self::answer(
                &self.cmd_tx,
                &self.live,
                &self.stats,
                id,
                Ending::Refused,
                LuaData::Nil,
                Some("every compute worker is busy and the queue is full".to_string()),
                Vec::new(),
            );
        }
        Ok(id)
    }

    /// Whether an entry module sits under one of the configured roots.
    fn module_is_allowed(&self, module: &str) -> bool {
        let normalized = module.replace('\\', "/").replace('.', "/");
        self.cfg.roots.iter().any(|root| {
            let root = root.trim_matches('/');
            normalized == root || normalized.starts_with(&format!("{root}/"))
        })
    }

    /// Ask a job to stop. Returns whether it was still live.
    pub fn cancel(&self, id: JobId) -> bool {
        let mut live = self.live.lock_recover();
        match live.get_mut(&id) {
            Some(entry) => {
                entry.cancelled = true;
                true
            }
            None => false,
        }
    }

    /// Rebuild every worker's VM before its next job. Called on reload.
    pub fn recycle(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Watch for jobs that blew their deadline.
    ///
    /// Owned by the bridge rather than driven from the driver's event loop, so
    /// the pool is complete on its own. It used to be a `select!` arm in
    /// `Driver::run`, which meant a bridge running under anything else — a
    /// test, a tool — silently never timed anything out.
    ///
    /// Exits when the engine's command channel closes, since at that point
    /// there is nobody left to answer.
    fn spawn_watchdog(&self) {
        let bridge = self.clone();
        std::thread::Builder::new()
            .name("oxigeon-compute-watchdog".to_string())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(100));
                if bridge.cmd_tx.is_closed() {
                    break;
                }
                bridge.reap_expired();
            })
            .expect("failed to spawn the compute watchdog");
    }

    /// Answer for every job whose deadline has passed.
    ///
    /// Called on a timer by the driver. The deadline unblocks the **caller**;
    /// it does not stop the job. With the compiler on there is no hook to
    /// interrupt one, so a job that overran is still burning its worker — that
    /// is what `wedged` counts, and why it is worth alerting on.
    pub fn reap_expired(&self) {
        let now = Instant::now();
        let expired: Vec<(JobId, bool)> = self
            .live
            .lock_recover()
            .iter()
            .filter(|(_, e)| e.deadline <= now)
            .map(|(id, e)| (*id, e.started.is_some()))
            .collect();

        for (id, was_running) in expired {
            if was_running {
                self.stats.lock_recover().wedged += 1;
                tracing::error!(
                    "compute: job {id} passed its deadline while running — the worker is \
                     still executing it and will not come back unless \
                     [compute] instruction_limit is set"
                );
            }
            Self::answer(
                &self.cmd_tx,
                &self.live,
                &self.stats,
                id,
                Ending::Timeout,
                LuaData::Nil,
                Some("the job did not finish before its deadline".to_string()),
                Vec::new(),
            );
        }
    }

    /// A view of the pool for `server_info()`.
    pub fn snapshot(&self) -> Snapshot {
        let live = self.live.lock_recover();
        Snapshot {
            workers: self.cfg.workers,
            queue_depth: self.cfg.queue_depth,
            instruction_limit: self.cfg.instruction_limit,
            in_flight: live.len(),
            running: live.values().filter(|e| e.started.is_some()).count(),
            stats: self.stats.lock_recover().clone(),
        }
    }

    pub fn default_deadline_ms(&self) -> u64 {
        self.cfg.default_deadline_ms
    }

    /// Limits for copying arguments out of the game VM.
    pub fn arg_limits(&self) -> Limits {
        Limits { depth: self.cfg.max_arg_depth, nodes: self.cfg.max_arg_nodes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge(cfg: ComputeConfig) -> Option<ComputeBridge> {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        ComputeBridge::start(cfg, PathBuf::from("."), PathBuf::from("."), tx)
    }

    #[test]
    fn compute_is_off_unless_asked_for() {
        assert!(bridge(ComputeConfig::default()).is_none());
    }

    #[test]
    fn only_modules_under_a_root_are_accepted() {
        let b = bridge(ComputeConfig { enabled: true, ..Default::default() }).unwrap();
        assert!(b.module_is_allowed("compute/pathfind"));
        assert!(b.module_is_allowed("compute.pathfind"));
        assert!(b.module_is_allowed("compute"));
        // The guardrail that stops `compute("daemons/world_d", ...)` producing
        // a baffling "attempt to call nil (global 'send')".
        assert!(!b.module_is_allowed("daemons/world_d"));
        assert!(!b.module_is_allowed("lib/commands"));
        // A prefix match must not let a sibling through.
        assert!(!b.module_is_allowed("computed/other"));
    }

    #[test]
    fn a_deadline_above_the_maximum_is_rejected_at_the_call_site() {
        let b = bridge(ComputeConfig {
            enabled: true,
            max_deadline_ms: 1_000,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            b.submit("compute/x".into(), "f".into(), LuaData::Nil, LuaData::Nil, Some(9_999)),
            Err(SubmitError::DeadlineTooLong { asked: 9_999, max: 1_000 })
        );
    }

    #[test]
    fn a_module_outside_the_roots_is_rejected_at_the_call_site() {
        let b = bridge(ComputeConfig { enabled: true, ..Default::default() }).unwrap();
        assert!(matches!(
            b.submit("daemons/world_d".into(), "reset".into(), LuaData::Nil, LuaData::Nil, None),
            Err(SubmitError::BadModule(_))
        ));
    }
}
