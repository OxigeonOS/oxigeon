//! Running long Lua computations off the game thread — and out of the game
//! process.
//!
//! The whole game runs on one Lua thread. Anything expensive on it — a
//! pathfind across a large map, generating an area, a simulation pass — freezes
//! every connected player for its duration. This is the escape hatch: hand the
//! work to a pool of `oxigeon-compute` child processes, each with its own LuaJIT
//! VM, and get the answer back later through a mudlib hook.
//!
//! It is deliberately shaped like [`crate::core::auth`], which solves the same
//! problem for Argon2: a bounded queue, a fixed pool, and a round trip through
//! [`LuaCommand`]. Two things differ. The work is arbitrary game code rather
//! than one fixed operation, so it has to be identified and loaded. And its
//! arguments and results must be *copied* between two VMs, because no Lua value
//! may leave the VM that made it — see [`oxigeon_lua::marshal`], framed for the
//! pipe by [`oxigeon_lua::wire`].
//!
//! # Why a process and not a thread
//!
//! Workers used to be threads in this process, and that cost two things.
//!
//! **They had to be the same Lua.** `mlua-sys` permits one Lua version per
//! binary, so a server built for Lua 5.5 — which is what makes debugging without
//! freezing the world possible — dragged its compute pool onto 5.5 too, giving
//! up the LuaJIT compiler on precisely the arithmetic-heavy work this facility
//! exists for.
//!
//! **They could not be stopped.** Rust cannot kill a thread. With no instruction
//! budget armed there is no hook to interrupt a runaway job, so one burned its
//! worker for the life of the *server*, permanently, and the only mitigation was
//! to count it. A process can be terminated: a job that overruns its deadline
//! now costs one job and a respawn.
//!
//! # The contract
//!
//! If [`ComputeBridge::submit`] returns an id, **exactly one** result is
//! delivered for it. If it returns an error, none is. Everything operational —
//! a full queue, a timeout, a cancel, a module that will not load, a job that
//! raises, a worker that died — arrives through the result path, because the
//! mudlib's cleanup is identical for all of them and making a caller handle "the
//! efun told me" and "the hook told me" as separate cases is how that cleanup
//! gets forgotten.

pub mod worker;

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

/// Copying a Lua value out of the game VM and back in again. Lives in
/// `oxigeon-lua` because the worker process needs exactly the same code.
pub use oxigeon_lua::marshal;
pub use oxigeon_lua::marshal::{Limits, LuaData, MarshalError};
pub use oxigeon_lua::vm::Ending;
pub use worker::WorkerPath;

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

pub(crate) struct Job {
    pub id: JobId,
    pub module: String,
    pub func: String,
    pub args: LuaData,
    pub deadline: Instant,
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
    /// Which worker picked it up, so a cancel or a kill reaches the right child.
    worker: Option<usize>,
}

/// Counters behind `server_info().compute`.
///
/// `wedged` is the one an operator should watch: it counts jobs that blew their
/// deadline while still running. Each of those killed and respawned a worker
/// process — recoverable, unlike the thread pool this replaced, but still a sign
/// that a job is doing something it should not.
#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub submitted: u64,
    pub completed: u64,
    pub failed: u64,
    pub timed_out: u64,
    pub refused: u64,
    pub cancelled: u64,
    pub wedged: u64,
    /// Worker processes started, including respawns after a kill or a crash.
    pub spawned: u64,
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
    /// Bumped on every reload; a worker is replaced before its next job.
    epoch: Arc<AtomicU64>,
    live: Arc<Mutex<HashMap<JobId, Live>>>,
    stats: Arc<Mutex<Stats>>,
    /// The child process behind each worker slot, for cancels and kills.
    workers: Arc<Vec<worker::Handle>>,
    cfg: Arc<ComputeConfig>,
}

impl ComputeBridge {
    /// Start the pool. Returns `None` when compute is disabled, which is what
    /// keeps the efuns unregistered, the feature free when unused, and — since
    /// workers are processes now — guarantees no child is ever spawned.
    pub fn start(
        cfg: ComputeConfig,
        mudlib: PathBuf,
        game: PathBuf,
        cmd_tx: UnboundedSender<LuaCommand>,
    ) -> Option<Self> {
        if !cfg.enabled {
            return None;
        }

        let count = cfg.workers.max(1);
        let (tx, rx) = sync_channel::<Job>(cfg.queue_depth.max(1));
        let bridge = Self {
            tx,
            cmd_tx: cmd_tx.clone(),
            next_id: Arc::new(AtomicU64::new(1)),
            epoch: Arc::new(AtomicU64::new(0)),
            live: Arc::new(Mutex::new(HashMap::new())),
            stats: Arc::new(Mutex::new(Stats::default())),
            workers: Arc::new((0..count).map(|_| worker::Handle::default()).collect()),
            cfg: Arc::new(cfg),
        };

        let rx = Arc::new(Mutex::new(rx));
        for n in 0..count {
            bridge.spawn_worker(n, rx.clone(), mudlib.clone(), game.clone());
        }
        bridge.spawn_watchdog();

        tracing::info!(
            "compute: {} worker process(es), queue {}, instruction limit {} ({})",
            count,
            bridge.cfg.queue_depth,
            bridge.cfg.instruction_limit,
            if bridge.cfg.instruction_limit > 0 {
                "compiler off, a runaway job stops itself"
            } else {
                "compiler on, a runaway job is killed at its deadline"
            }
        );

        Some(bridge)
    }

    /// One host thread per worker slot: pull a job, hand it to that slot's child
    /// process, wait for the answer.
    ///
    /// The thread is cheap and does no Lua; all it does is own one pipe. Keeping
    /// a thread per child is what lets the read be a plain blocking read instead
    /// of a poll loop over every worker.
    fn spawn_worker(&self, n: usize, rx: Arc<Mutex<Receiver<Job>>>, mudlib: PathBuf, game: PathBuf) {
        let cfg = self.cfg.clone();
        let epoch = self.epoch.clone();
        let live = self.live.clone();
        let stats = self.stats.clone();
        let cmd_tx = self.cmd_tx.clone();
        let handle = self.workers[n].clone();

        std::thread::Builder::new()
            .name(format!("oxigeon-compute-{n}"))
            .spawn(move || {
                // Built lazily and replaced whenever the epoch moves, so compute
                // costs nothing until it is used and a reload is picked up
                // without any `package.loaded` surgery. Throwing the whole VM
                // away is always safe here precisely because it holds no state
                // anyone is allowed to depend on — which is the property the
                // game VM lacks.
                let mut at_epoch: Option<u64> = None;
                // The reading end of the current child's pipe. Held here rather
                // than in the slot so a blocking read cannot stop a cancel or a
                // kill — see `worker`'s module docs.
                let mut reader: Option<worker::Reader> = None;

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
                                entry.worker = Some(n);
                                true
                            }
                            _ => false,
                        }
                    };
                    if !still_wanted {
                        continue;
                    }

                    let current = epoch.load(Ordering::Relaxed);
                    if at_epoch != Some(current) || !handle.is_running() {
                        handle.shut_down();
                        reader = None;
                        at_epoch = None;
                    }
                    if reader.is_none() {
                        stats.lock_recover().spawned += 1;
                        match handle.start(&cfg, &mudlib, &game, (n as u64) ^ (current << 8)) {
                            Ok(r) => {
                                reader = Some(r);
                                at_epoch = Some(current);
                            }
                            Err(e) => {
                                tracing::error!(
                                    "compute: could not start a worker process: {e}"
                                );
                                Self::answer(
                                    &cmd_tx, &live, &stats, job.id,
                                    Ending::LoadError,
                                    LuaData::Nil,
                                    Some(format!("compute worker could not start: {e}")),
                                    Vec::new(),
                                );
                                // Do not spin: with no worker binary every job
                                // fails the same way, and the log line above is
                                // the one that matters.
                                std::thread::sleep(Duration::from_millis(250));
                                continue;
                            }
                        }
                    }

                    let out = handle.run(reader.as_mut().expect("just started"), &job);
                    if handle.is_broken() {
                        // The child died mid-job — killed at its deadline, or it
                        // crashed. Drop it so the next job gets a fresh one;
                        // whoever killed it has usually answered already, and
                        // `answer` is a no-op if so.
                        handle.shut_down();
                        reader = None;
                        at_epoch = None;
                    }
                    Self::answer(
                        &cmd_tx, &live, &stats, job.id,
                        out.ending, out.value, out.error, out.logs,
                    );
                }

                handle.shut_down();
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
                worker: None,
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
    ///
    /// The flag alone only reaches a job that checks `compute_cancelled()` or
    /// runs under a budget, so a cancel for a job already in a worker is also
    /// sent down that worker's pipe.
    pub fn cancel(&self, id: JobId) -> bool {
        let mut live = self.live.lock_recover();
        match live.get_mut(&id) {
            Some(entry) => {
                entry.cancelled = true;
                if let Some(w) = entry.worker.and_then(|n| self.workers.get(n)) {
                    w.cancel(id);
                }
                true
            }
            None => false,
        }
    }

    /// Replace every worker's VM before its next job. Called on reload.
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

    /// Answer for every job whose deadline has passed, and kill the worker of
    /// any that was still running.
    ///
    /// The deadline unblocks the caller. What is new since compute moved out of
    /// process is that it also *ends the job*: with no budget armed there is no
    /// hook inside the VM to interrupt one, and the only thing that reliably
    /// stops a runaway `while true do end` is terminating the process running
    /// it. That is what `wedged` counts now — a worker killed and replaced,
    /// rather than a worker lost for the life of the server.
    pub fn reap_expired(&self) {
        let now = Instant::now();
        let expired: Vec<(JobId, Option<usize>)> = self
            .live
            .lock_recover()
            .iter()
            .filter(|(_, e)| e.deadline <= now)
            .map(|(id, e)| (*id, e.started.map(|_| e.worker.unwrap_or(usize::MAX))))
            .collect();

        for (id, running_on) in expired {
            if let Some(n) = running_on {
                self.stats.lock_recover().wedged += 1;
                tracing::error!(
                    "compute: job {id} passed its deadline while running — killing worker {n}"
                );
                if let Some(w) = self.workers.get(n) {
                    w.kill();
                }
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

    /// How many worker child processes are alive right now. For tests that need
    /// to assert compute spawned nothing.
    pub fn live_worker_count(&self) -> usize {
        self.workers.iter().filter(|w| w.is_running()).count()
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
