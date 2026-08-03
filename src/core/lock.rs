//! Lock acquisition that survives poisoning.
//!
//! A `std` lock is poisoned forever once a thread panics while holding it, and
//! every later `.unwrap()` on it panics too. In a driver where the Lua thread
//! takes the `session_handler` lock on nearly every efun, that turns one
//! recoverable panic into a permanent outage: the Lua thread dies on its next
//! efun and the process stays up serving a game that no longer runs. There were
//! 24 such `.unwrap()`s in `efuns.rs` alone.
//!
//! Poisoning is a warning, not a corruption: it means *some* thread panicked
//! mid-update, so the data behind the lock may be inconsistent. For this
//! codebase the guarded values are a session map, a config snapshot and a few
//! counters — carrying on with one of those in an odd state is plainly better
//! than taking the whole game down. So these recover, and say so.
//!
//! Recovery is reported loudly the first time and quietly after that: the first
//! one is the incident, and the thousand that follow are the same incident.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

static REPORTED: AtomicBool = AtomicBool::new(false);

#[cold]
#[track_caller]
fn report(kind: &str) {
    let at = std::panic::Location::caller();
    if REPORTED.swap(true, Ordering::Relaxed) {
        tracing::debug!("recovered a poisoned {} at {}", kind, at);
    } else {
        tracing::error!(
            "recovered a poisoned {} at {} — a thread panicked while holding it, so the \
             state behind it may be inconsistent. Continuing rather than taking the game \
             down with it; later recoveries are logged at debug.",
            kind,
            at
        );
    }
}

pub trait RwLockExt<T: ?Sized> {
    /// Read, recovering if the lock is poisoned.
    fn read_recover(&self) -> RwLockReadGuard<'_, T>;
    /// Write, recovering if the lock is poisoned.
    fn write_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> RwLockExt<T> for RwLock<T> {
    #[track_caller]
    fn read_recover(&self) -> RwLockReadGuard<'_, T> {
        match self.read() {
            Ok(g) => g,
            Err(poisoned) => {
                report("RwLock (read)");
                poisoned.into_inner()
            }
        }
    }

    #[track_caller]
    fn write_recover(&self) -> RwLockWriteGuard<'_, T> {
        match self.write() {
            Ok(g) => g,
            Err(poisoned) => {
                report("RwLock (write)");
                poisoned.into_inner()
            }
        }
    }
}

pub trait MutexExt<T: ?Sized> {
    /// Lock, recovering if the mutex is poisoned.
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> MutexExt<T> for Mutex<T> {
    #[track_caller]
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        match self.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                report("Mutex");
                poisoned.into_inner()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn poison_rwlock() -> Arc<RwLock<Vec<i32>>> {
        let lock = Arc::new(RwLock::new(vec![1, 2, 3]));
        let clone = lock.clone();
        // A panic while holding the write lock is exactly the scenario: the
        // value is left mid-update.
        let _ = std::thread::spawn(move || {
            let mut guard = clone.write().unwrap();
            guard.push(4);
            panic!("deliberate panic while holding the lock");
        })
        .join();
        assert!(lock.read().is_err(), "the lock should now be poisoned");
        lock
    }

    #[test]
    fn a_poisoned_rwlock_can_still_be_read() {
        let lock = poison_rwlock();
        assert_eq!(*lock.read_recover(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_poisoned_rwlock_can_still_be_written() {
        let lock = poison_rwlock();
        lock.write_recover().push(5);
        assert_eq!(*lock.read_recover(), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_poisoned_mutex_can_still_be_locked() {
        let lock = Arc::new(Mutex::new(0u32));
        let clone = lock.clone();
        let _ = std::thread::spawn(move || {
            let mut guard = clone.lock().unwrap();
            *guard = 7;
            panic!("deliberate panic while holding the mutex");
        })
        .join();
        assert!(lock.lock().is_err());
        assert_eq!(*lock.lock_recover(), 7);
    }

    #[test]
    fn an_unpoisoned_lock_behaves_normally() {
        let lock = RwLock::new(1);
        assert_eq!(*lock.read_recover(), 1);
        *lock.write_recover() = 2;
        assert_eq!(*lock.read_recover(), 2);
    }
}
