//! One worker slot: the `oxigeon-compute` child process behind it, and the
//! pipe to it.
//!
//! Everything here is deliberately dumb. The slot owns a child, writes a job
//! frame, and blocks reading until the answer comes back. Policy — deadlines,
//! cancels, the queue, who gets answered — stays in [`super`], and the only
//! reason this is a separate file is that process plumbing on two platforms is
//! bulky enough to bury it.
//!
//! # Who touches what
//!
//! `cancel` and `kill` come from *other* threads — the watchdog, and the game
//! thread via `compute_cancel` — and they have to get through **while a job is
//! running**, which is the only time they are worth anything. So the child and
//! its stdin sit behind a mutex, and the reading end deliberately does not: it
//! is handed to the host thread by [`Handle::start`] and stays there. Holding
//! one lock across the blocking read would block exactly the two calls the lock
//! exists to serialize, and a wedged job would then be unkillable — which is the
//! bug this whole rewrite is meant to fix.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use oxigeon_lua::vm::Outcome;
use oxigeon_lua::wire::{read_frame, write_frame, ToServer, ToWorker};
use oxigeon_lua::{ComputeSettings, Ending, LuaData};

use crate::config::server_config::ComputeConfig;
use crate::core::lock::MutexExt;

/// Where the worker binary is, and why we think so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerPath {
    /// `[compute] worker_path` said so.
    Configured(PathBuf),
    /// Next to the running server binary, which is how a release is laid out.
    BesideServer(PathBuf),
}

impl WorkerPath {
    pub fn path(&self) -> &Path {
        match self {
            Self::Configured(p) | Self::BesideServer(p) => p,
        }
    }
}

/// Resolve the worker binary.
///
/// Beside the server rather than on `PATH`: a compute worker is part of *this*
/// build — it shares the wire protocol and the sandbox with it — and picking up
/// some other version from a search path is the kind of mismatch that shows up
/// as a decode error under load.
pub fn resolve_worker(cfg: &ComputeConfig) -> std::io::Result<WorkerPath> {
    if let Some(p) = &cfg.worker_path {
        let p = PathBuf::from(p);
        return if p.is_file() {
            Ok(WorkerPath::Configured(p))
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("[compute] worker_path points at '{}', which is not a file", p.display()),
            ))
        };
    }

    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "the server binary has no directory")
    })?;
    let candidate = dir.join(format!("oxigeon-compute{}", std::env::consts::EXE_SUFFIX));
    if candidate.is_file() {
        return Ok(WorkerPath::BesideServer(candidate));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!(
            "no compute worker at '{}'. It is a separate binary because it links a \
             different Lua: build it with `cargo build --release -p oxigeon-compute`, \
             or set [compute] worker_path",
            candidate.display()
        ),
    ))
}

/// The live child of one worker slot. Not its reading end — see the module docs.
struct Live {
    child: Child,
    stdin: ChildStdin,
    /// Set when a frame could not be read or written, or when the child was
    /// killed. The slot is replaced before its next job.
    broken: bool,
}

/// The reading end of a worker's pipe, owned by the one thread that reads it.
pub(crate) type Reader = BufReader<ChildStdout>;

/// A worker slot. Cloneable, and every clone refers to the same child.
#[derive(Clone, Default)]
pub(crate) struct Handle {
    live: Arc<Mutex<Option<Live>>>,
}

impl Handle {
    pub(crate) fn is_running(&self) -> bool {
        self.live.lock_recover().is_some()
    }

    pub(crate) fn is_broken(&self) -> bool {
        self.live.lock_recover().as_ref().is_none_or(|l| l.broken)
    }

    /// Spawn the child and complete the handshake. Returns once it is ready for
    /// jobs, so a failure to *build a VM* is reported here rather than as a
    /// mysteriously failing first job.
    ///
    /// The [`Reader`] goes to the caller and must be kept for as long as the
    /// worker is used: it is the only reading end, held outside the lock so a
    /// blocking read cannot stop a cancel or a kill.
    pub(crate) fn start(
        &self,
        cfg: &ComputeConfig,
        mudlib: &Path,
        game: &Path,
        salt: u64,
    ) -> std::io::Result<Reader> {
        let bin = resolve_worker(cfg)?;
        let mut child = Command::new(bin.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr is inherited on purpose: a worker's own diagnostics should
            // land in the server's log, and there is nobody else to read them.
            .stderr(Stdio::inherit())
            .spawn()?;

        let mut stdin = child.stdin.take().expect("stdin was piped");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout was piped"));

        let hello = ToWorker::Hello {
            settings: settings_of(cfg),
            mudlib: mudlib.to_string_lossy().into_owned(),
            game: game.to_string_lossy().into_owned(),
            salt,
        };
        write_frame(&mut stdin, &hello.encode())?;

        match read_frame(&mut stdout)? {
            Some(body) => match ToServer::decode(&body)? {
                ToServer::Ready => {}
                ToServer::Broken { error } => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(std::io::Error::other(error));
                }
                other => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("worker answered the handshake with {other:?}"),
                    ));
                }
            },
            None => {
                let _ = child.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the worker exited during the handshake",
                ));
            }
        }

        tracing::debug!("compute: worker started ({:?})", bin);
        *self.live.lock_recover() = Some(Live { child, stdin, broken: false });
        Ok(stdout)
    }

    /// Send a job and wait for its answer.
    ///
    /// Called only from the host thread that owns `reader`, which is what makes
    /// the "one reader" rule hold. A dead or killed child comes back as an
    /// [`Ending`] rather than an error, because the caller has exactly one way to
    /// report anything.
    pub(crate) fn run(&self, reader: &mut Reader, job: &super::Job) -> Outcome {
        let deadline_ms = job
            .deadline
            .saturating_duration_since(std::time::Instant::now())
            .as_millis()
            .min(u64::MAX as u128) as u64;

        let frame = ToWorker::Job {
            id: job.id,
            module: job.module.clone(),
            func: job.func.clone(),
            args: job.args.clone(),
            deadline_ms,
        }
        .encode();

        // The write and the read are separate locks. Holding one across the
        // whole job would block `cancel`, which is the one thing that has to get
        // through *while* a job runs.
        if let Err(e) = self.write(&frame) {
            return died(format!("could not send the job to its worker: {e}"));
        }

        loop {
            match read_one(reader) {
                Ok(Some(ToServer::Done { id, ending, value, error, logs })) => {
                    if id != job.id {
                        // Only possible if the stream desynchronized, which
                        // means every later frame is suspect too.
                        self.mark_broken();
                        return died(format!(
                            "worker answered for job {id} while running job {}",
                            job.id
                        ));
                    }
                    return Outcome { ending, value, error, logs };
                }
                // Ready or Broken outside the handshake: ignore Ready, treat a
                // Broken as what it is.
                Ok(Some(ToServer::Ready)) => continue,
                Ok(Some(ToServer::Broken { error })) => {
                    self.mark_broken();
                    return died(format!("worker reported itself broken: {error}"));
                }
                Ok(None) => {
                    self.mark_broken();
                    return died(
                        "the worker process ended without answering — it was killed at its \
                         deadline, or it crashed"
                            .to_string(),
                    );
                }
                Err(e) => {
                    self.mark_broken();
                    return died(format!("lost contact with the worker: {e}"));
                }
            }
        }
    }

    /// Tell the child to abandon the running job. Best effort by design: the
    /// caller's real lever is [`Handle::kill`].
    pub(crate) fn cancel(&self, id: u64) {
        let frame = ToWorker::Cancel { id }.encode();
        if let Err(e) = self.write(&frame) {
            tracing::debug!("compute: could not deliver a cancel: {e}");
        }
    }

    /// Terminate the child. The host thread's blocking read then returns end of
    /// stream, and the slot is replaced before its next job.
    pub(crate) fn kill(&self) {
        if let Some(live) = self.live.lock_recover().as_mut() {
            live.broken = true;
            let _ = live.child.kill();
        }
    }

    /// Close the pipe and reap the child.
    ///
    /// Dropping stdin is what ends a *healthy* worker: it sees end of stream and
    /// exits. `kill` is for one that will not.
    pub(crate) fn shut_down(&self) {
        let Some(live) = self.live.lock_recover().take() else {
            return;
        };
        let mut live = live;
        drop(live.stdin);
        // A worker that is mid-job will not notice the closed pipe until it
        // finishes, and a wedged one never will, so do not wait on it here —
        // that would move the hang into the server's shutdown path.
        let _ = live.child.kill();
        let _ = live.child.wait();
    }

    fn write(&self, frame: &[u8]) -> std::io::Result<()> {
        let mut guard = self.live.lock_recover();
        let live = guard
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "no worker"))?;
        let r = write_frame(&mut live.stdin, frame);
        if r.is_err() {
            live.broken = true;
        }
        r
    }

    fn mark_broken(&self) {
        if let Some(live) = self.live.lock_recover().as_mut() {
            live.broken = true;
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // Only the last clone owns the child; `Arc::get_mut` succeeds exactly
        // then. Without this a server that drops its bridge would leave worker
        // processes running with a closed pipe.
        if Arc::get_mut(&mut self.live).is_some() {
            self.shut_down();
        }
    }
}

/// Read one message. Free of the slot's lock on purpose — see the module docs.
fn read_one(reader: &mut Reader) -> std::io::Result<Option<ToServer>> {
    match read_frame(reader)? {
        Some(body) => Ok(Some(ToServer::decode(&body)?)),
        None => Ok(None),
    }
}

/// The VM-shaping subset of `[compute]` that a worker is told about.
fn settings_of(cfg: &ComputeConfig) -> ComputeSettings {
    ComputeSettings {
        instruction_limit: cfg.instruction_limit,
        memory_mb: cfg.memory_mb,
        max_arg_depth: cfg.max_arg_depth,
        max_arg_nodes: cfg.max_arg_nodes,
    }
}

/// The outcome for a job whose worker is gone.
///
/// `Ending::Error` rather than a variant of its own: from the mudlib's side "the
/// job did not produce an answer" is one case with one cleanup, and adding a
/// `worker_died` kind would give callers a new branch to forget.
fn died(error: String) -> Outcome {
    Outcome { ending: Ending::Error, value: LuaData::Nil, error: Some(error), logs: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(path: Option<String>) -> ComputeConfig {
        ComputeConfig { enabled: true, worker_path: path, ..Default::default() }
    }

    #[test]
    fn a_configured_worker_path_is_used_as_given() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin = dir.path().join("my-worker");
        std::fs::write(&bin, b"not really a binary").unwrap();
        assert_eq!(
            resolve_worker(&cfg_with(Some(bin.to_string_lossy().into_owned()))).unwrap(),
            WorkerPath::Configured(bin)
        );
    }

    /// The failure an operator will actually hit: compute turned on, the worker
    /// never built. It has to name the command, because "No such file or
    /// directory" from a path they never typed is unactionable.
    #[test]
    fn a_missing_worker_says_how_to_build_one() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("nope");
        let e = resolve_worker(&cfg_with(Some(missing.to_string_lossy().into_owned())))
            .expect_err("a worker_path that is not a file must not resolve");
        assert!(e.to_string().contains("worker_path"), "{e}");

        // And with nothing configured, the message has to explain *why* there is
        // a second binary at all.
        let e = match resolve_worker(&cfg_with(None)) {
            Err(e) => e.to_string(),
            // Under `cargo test` the binary may genuinely sit beside the test
            // executable; then there is no error to inspect and nothing to say.
            Ok(_) => return,
        };
        assert!(e.contains("cargo build"), "{e}");
        assert!(e.contains("different Lua"), "{e}");
    }

    /// A directory is not a worker. `is_file` rather than `exists`, or spawning
    /// fails much later with a far worse message.
    #[test]
    fn a_directory_is_not_a_worker() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(resolve_worker(&cfg_with(Some(dir.path().to_string_lossy().into_owned()))).is_err());
    }
}
