//! Password hashing, off the game thread.
//!
//! Argon2 is a deliberately expensive KDF — measured on this machine at ~370 ms
//! to hash and ~386 ms to verify in a debug build. `authenticate` and
//! `create_account` used to run it inline on the Lua thread, which is the one
//! thread the entire game runs on: every login froze the world for that long,
//! and since login happens *before* authentication, anyone who could open a
//! socket could freeze it at will just by spamming attempts.
//!
//! So the work moves to a small fixed pool and the answer comes back to Lua as
//! a [`LuaCommand`], the same round trip the timer efuns use. Two things follow
//! from the pool being fixed:
//!
//! - the queue is bounded, so a flood of attempts is refused rather than
//!   spawning unbounded work, and
//! - repeated failures from one address are refused before any hashing happens,
//!   which is what makes the refusal cheap enough to be worth doing.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::core::lock::MutexExt;
use crate::core::scripting::engine::LuaCommand;
use crate::domain::models::DieselAccountStore;

/// Which operation the worker should perform.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthKind {
    Authenticate,
    CreateAccount,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authenticate => "authenticate",
            Self::CreateAccount => "create_account",
        }
    }
}

/// Why a request was refused without being run.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Every worker is busy and the queue is full.
    Busy,
    /// Too many recent failures from this address. Carries the seconds left.
    RateLimited(u64),
}

impl Refused {
    /// The message Lua receives as the `error` argument.
    pub fn message(&self) -> String {
        match self {
            Self::Busy => "The server is busy authenticating. Please try again.".to_string(),
            Self::RateLimited(secs) => format!(
                "Too many failed attempts. Try again in {} seconds.",
                secs.max(&1)
            ),
        }
    }
}

struct Job {
    session_id: String,
    kind: AuthKind,
    username: String,
    password: String,
    /// Resolved once, on submit, so the worker never touches the session table.
    peer: Option<IpAddr>,
}

/// Consecutive failed attempts from one address.
#[derive(Default)]
struct Strikes {
    count: u32,
    /// When the lockout, if any, expires.
    until: Option<Instant>,
}

/// Failed attempts before an address is locked out.
const MAX_STRIKES: u32 = 5;
/// How long a locked-out address stays locked out.
const LOCKOUT: Duration = Duration::from_secs(30);
/// Queued requests allowed before new ones are refused. Each one costs a worker
/// several hundred milliseconds, so a deep queue would only mean players
/// waiting a long time to be told the server is busy.
const QUEUE_DEPTH: usize = 32;

/// Handle to the pool. Cloneable; dropping the last one stops the workers.
#[derive(Clone)]
pub struct AuthWorker {
    tx: SyncSender<Job>,
    strikes: Arc<Mutex<HashMap<IpAddr, Strikes>>>,
}

impl AuthWorker {
    /// Start `threads` workers. They exit when the last handle is dropped.
    pub fn start(
        store: Arc<DieselAccountStore>,
        cmd_tx: UnboundedSender<LuaCommand>,
        threads: usize,
    ) -> Self {
        let (tx, rx) = sync_channel::<Job>(QUEUE_DEPTH);
        let rx = Arc::new(Mutex::new(rx));
        let strikes: Arc<Mutex<HashMap<IpAddr, Strikes>>> = Arc::new(Mutex::new(HashMap::new()));

        for n in 0..threads.max(1) {
            let rx = rx.clone();
            let store = store.clone();
            let cmd_tx = cmd_tx.clone();
            let strikes = strikes.clone();
            std::thread::Builder::new()
                .name(format!("oxigeon-auth-{n}"))
                .spawn(move || loop {
                    // Held only for the recv, never across the hash.
                    let job = {
                        // A worker that panicked mid-hash poisons this; the
                        // queue itself is still sound, so carry on rather than
                        // silently losing a worker for the process's remaining
                        // life.
                        let guard = rx.lock_recover();
                        match guard.recv() {
                            Ok(job) => job,
                            Err(_) => break, // every sender dropped
                        }
                    };
                    let msg = run_job(&store, &strikes, job);
                    if cmd_tx.send(msg).is_err() {
                        break; // the Lua thread is gone
                    }
                })
                .expect("failed to spawn an auth worker");
        }

        Self { tx, strikes }
    }

    /// Queue a request. Returns `Err` if it was refused outright — the caller
    /// reports that to the player itself, because no worker will.
    pub fn submit(
        &self,
        session_id: String,
        kind: AuthKind,
        username: String,
        password: String,
        peer: Option<IpAddr>,
    ) -> Result<(), Refused> {
        // Checked before queueing so a locked-out address costs nothing but a
        // map lookup — the point of the limit is to not spend Argon2 on it.
        if let Some(ip) = peer {
            if let Some(left) = self.lockout_remaining(ip) {
                return Err(Refused::RateLimited(left.as_secs()));
            }
        }

        let job = Job { session_id, kind, username, password, peer };
        match self.tx.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(Refused::Busy),
            Err(TrySendError::Disconnected(_)) => Err(Refused::Busy),
        }
    }

    fn lockout_remaining(&self, ip: IpAddr) -> Option<Duration> {
        let mut map = self.strikes.lock_recover();
        let entry = map.get_mut(&ip)?;
        match entry.until {
            Some(until) if until > Instant::now() => Some(until - Instant::now()),
            Some(_) => {
                // Expired — clear it so the next failure starts a fresh count.
                entry.count = 0;
                entry.until = None;
                None
            }
            None => None,
        }
    }

    /// Forget an address's failures. Called when its session disconnects, so a
    /// lockout cannot outlive the connection that earned it by more than the
    /// window.
    pub fn forget(&self, peer: Option<IpAddr>) {
        if let Some(ip) = peer {
            let mut map = self.strikes.lock_recover();
            if map.get(&ip).is_some_and(|s| s.until.is_none()) {
                map.remove(&ip);
            }
        }
    }
}

/// Do the expensive part and build the reply. Runs on a worker thread.
fn run_job(
    store: &DieselAccountStore,
    strikes: &Mutex<HashMap<IpAddr, Strikes>>,
    job: Job,
) -> LuaCommand {
    let Job { session_id, kind, username, password, peer } = job;

    let (account, error) = match kind {
        AuthKind::Authenticate => match store.authenticate(&username, &password) {
            Ok(acct) => (Some(acct.to_lua_table()), None),
            Err(_) => {
                // Deliberately not distinguishing "no such user" from "wrong
                // password" in what goes back to the client.
                (None, Some("Invalid username or password.".to_string()))
            }
        },
        AuthKind::CreateAccount => match store.create(&username, &password) {
            Ok(acct) => (Some(acct.to_lua_table()), None),
            Err(e) => {
                tracing::warn!("create_account failed for {:?}: {}", username, e);
                (
                    None,
                    Some(
                        "Could not create account. The name may already be taken, \
                         or the password is too short."
                            .to_string(),
                    ),
                )
            }
        },
    };

    record_outcome(strikes, peer, account.is_some());

    LuaCommand::AuthResult {
        session_id,
        kind: kind.as_str(),
        account,
        error,
    }
}

fn record_outcome(strikes: &Mutex<HashMap<IpAddr, Strikes>>, peer: Option<IpAddr>, ok: bool) {
    let Some(ip) = peer else { return };
    let mut map = strikes.lock_recover();
    if ok {
        map.remove(&ip);
        return;
    }
    let entry = map.entry(ip).or_default();
    entry.count += 1;
    if entry.count >= MAX_STRIKES {
        entry.until = Some(Instant::now() + LOCKOUT);
        tracing::warn!(
            "auth: locking out {} for {:?} after {} consecutive failures",
            ip,
            LOCKOUT,
            entry.count
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(n: u8) -> Option<IpAddr> {
        Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, n)))
    }

    fn empty_strikes() -> Mutex<HashMap<IpAddr, Strikes>> {
        Mutex::new(HashMap::new())
    }

    #[test]
    fn failures_below_the_threshold_do_not_lock_anything_out() {
        let strikes = empty_strikes();
        for _ in 0..MAX_STRIKES - 1 {
            record_outcome(&strikes, ip(1), false);
        }
        let map = strikes.lock().unwrap();
        assert!(map[&ip(1).unwrap()].until.is_none());
    }

    #[test]
    fn enough_failures_lock_the_address_out() {
        let strikes = empty_strikes();
        for _ in 0..MAX_STRIKES {
            record_outcome(&strikes, ip(1), false);
        }
        let map = strikes.lock().unwrap();
        assert!(map[&ip(1).unwrap()].until.is_some());
    }

    #[test]
    fn a_success_clears_the_count() {
        let strikes = empty_strikes();
        for _ in 0..MAX_STRIKES - 1 {
            record_outcome(&strikes, ip(1), false);
        }
        record_outcome(&strikes, ip(1), true);
        assert!(strikes.lock().unwrap().is_empty());

        // ...and the next failure starts from one, not from the old total.
        record_outcome(&strikes, ip(1), false);
        let map = strikes.lock().unwrap();
        assert_eq!(map[&ip(1).unwrap()].count, 1);
        assert!(map[&ip(1).unwrap()].until.is_none());
    }

    #[test]
    fn one_address_locking_out_does_not_affect_another() {
        let strikes = empty_strikes();
        for _ in 0..MAX_STRIKES {
            record_outcome(&strikes, ip(1), false);
        }
        record_outcome(&strikes, ip(2), false);
        let map = strikes.lock().unwrap();
        assert!(map[&ip(1).unwrap()].until.is_some());
        assert!(map[&ip(2).unwrap()].until.is_none());
    }

    #[test]
    fn a_refusal_says_how_long_to_wait_and_never_says_zero() {
        assert!(Refused::RateLimited(0).message().contains("1 seconds"));
        assert!(Refused::RateLimited(12).message().contains("12 seconds"));
    }
}
