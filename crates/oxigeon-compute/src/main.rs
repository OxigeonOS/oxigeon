//! One compute worker: a LuaJIT VM that runs jobs handed to it on stdin.
//!
//! The server spawns one of these per configured worker and talks to it over
//! the child's own stdin and stdout, framed by [`oxigeon_lua::wire`]. It exists
//! as a separate process for two reasons, and the second one is the better one:
//!
//! 1. **It can be a different Lua.** The game thread may be Lua 5.5, for a debug
//!    hook that can yield; a compute job wants LuaJIT's compiler. One binary
//!    cannot link both.
//! 2. **It can be killed.** Rust cannot kill a thread, so the in-process pool
//!    this replaces had a documented hole: a runaway job with no instruction
//!    budget burned its worker for the life of the *server*. A process can be
//!    terminated and replaced, so a wedged job now costs one job.
//!
//! # stdout is the protocol
//!
//! Nothing here may print. `println!` in a worker would be parsed as a frame
//! header and desynchronize the stream — the failure would look like a corrupt
//! job result rather than a stray debug line. Anything a job wants to say goes
//! through `compute_log`, comes back inside its result, and is journalled by the
//! server. Diagnostics from the worker itself go to stderr, which the server
//! inherits.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxigeon_lua::vm::{self, ComputeVm};
use oxigeon_lua::wire::{read_frame, write_frame, ToServer, ToWorker};
use oxigeon_lua::LuaData;

fn main() {
    // Exit code 2 is "the worker could not start at all", distinct from a job
    // that failed. The server reports it and does not retry in a loop.
    if let Err(e) = run() {
        eprintln!("oxigeon-compute: {e}");
        std::process::exit(2);
    }
}

/// What the reader thread hands to the Lua thread.
enum Incoming {
    Job { id: u64, module: String, func: String, args: LuaData, deadline_ms: u64 },
}

fn run() -> io::Result<()> {
    // `io::stdin()`'s guard is not `Send`, and the reader below runs on its own
    // thread, so take the handle itself rather than a lock on it. Nothing else
    // in this process reads stdin.
    let mut stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    // ── handshake ────────────────────────────────────────────────────────
    let Some(body) = read_frame(&mut stdin)? else {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no handshake"));
    };
    let ToWorker::Hello { settings, mudlib, game, salt } = ToWorker::decode(&body)? else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "first frame was not a hello"));
    };

    let machine = match vm::build(&settings, mudlib.as_ref(), game.as_ref(), salt) {
        Ok(m) => m,
        Err(e) => {
            // Report before dying, so the server can say *why* rather than
            // reporting a worker that vanished.
            let _ = write_frame(&mut stdout, &ToServer::Broken { error: e.to_string() }.encode());
            return Err(io::Error::other(format!("could not build the Lua VM: {e}")));
        }
    };
    write_frame(&mut stdout, &ToServer::Ready.encode())?;

    // ── reader thread ────────────────────────────────────────────────────
    //
    // Jobs and cancels arrive on the same pipe, and a cancel is only useful
    // while a job is *running* — that is, while the Lua thread is inside
    // `machine.run` and cannot read anything. So stdin is drained by its own
    // thread, which puts jobs on a channel and sets the cancel flag directly.
    let cancel = machine.cancel_flag();
    let (tx, rx) = sync_channel::<Incoming>(1);
    std::thread::Builder::new()
        .name("compute-stdin".to_string())
        .spawn(move || read_stdin(stdin, tx, cancel))
        .expect("failed to spawn the worker's stdin reader");

    // ── the Lua thread ───────────────────────────────────────────────────
    serve(&machine, &rx, &mut stdout)
}

/// Drain stdin forever: jobs onto the channel, cancels straight onto the flag.
///
/// A cancel is deliberately not queued. Queueing it behind the job it is meant
/// to interrupt is the one ordering that makes it useless.
fn read_stdin(mut stdin: impl Read, tx: SyncSender<Incoming>, cancel: Arc<AtomicBool>) {
    loop {
        let frame = match read_frame(&mut stdin) {
            Ok(Some(f)) => f,
            Ok(None) => leave(0),
            Err(e) => {
                eprintln!("oxigeon-compute: stdin: {e}");
                leave(1);
            }
        };
        match ToWorker::decode(&frame) {
            Ok(ToWorker::Job { id, module, func, args, deadline_ms }) => {
                if tx.send(Incoming::Job { id, module, func, args, deadline_ms }).is_err() {
                    leave(0); // the Lua thread is gone
                }
            }
            // The id is not checked against the running job: the server sends at
            // most one job at a time to a worker, so the only job a cancel can
            // refer to is the one in flight. Checking would need the reader to
            // know what the Lua thread is doing, which is the coupling this
            // split exists to avoid.
            Ok(ToWorker::Cancel { .. }) => cancel.store(true, Ordering::Relaxed),
            Ok(ToWorker::Hello { .. }) => {
                eprintln!("oxigeon-compute: a second hello was ignored");
            }
            Err(e) => {
                eprintln!("oxigeon-compute: {e}");
                leave(1);
            }
        }
    }
}

/// End the process now, whatever the Lua thread is doing.
///
/// Reached when stdin closes, which means the server is gone — killed, crashed,
/// or shut down. Returning and letting `main` unwind would only work for a
/// worker that is *idle*: one in the middle of an uninterruptible job would
/// never look at the channel again and would outlive its server as an orphan
/// burning a core. Nobody is waiting for that answer, so do not finish it.
fn leave(code: i32) -> ! {
    std::process::exit(code)
}

/// Run jobs until stdin closes.
fn serve(machine: &ComputeVm, rx: &Receiver<Incoming>, out: &mut impl Write) -> io::Result<()> {
    while let Ok(Incoming::Job { id, module, func, args, deadline_ms }) = rx.recv() {
        // Measured from here rather than from a timestamp the server sent: the
        // two processes share no clock. The server's own deadline still governs
        // when the caller is answered, so this can only make the worker give up
        // early, never late.
        let deadline = (deadline_ms > 0).then(|| Instant::now() + Duration::from_millis(deadline_ms));
        let outcome = machine.run(&module, &func, &args, deadline);

        write_frame(
            out,
            &ToServer::Done {
                id,
                ending: outcome.ending,
                value: outcome.value,
                error: outcome.error,
                logs: outcome.logs,
            }
            .encode(),
        )?;
    }
    Ok(())
}
