//! Argon2 used to run on the Lua thread, so every login froze the whole game
//! for a few hundred milliseconds — before authentication, which made it a
//! denial of service anyone with a socket could trigger by spamming attempts.
//!
//! These drive the real engine: `authenticate` is called from inside the VM,
//! and the assertion is that the Lua thread keeps answering while the hash is
//! still running. A test of the worker pool alone could not tell the difference.

use std::time::{Duration, Instant};

use crate::common::RealVm;

/// The property the whole change exists for: the efun returns before the hash
/// finishes, and the game thread keeps serving in between.
///
/// Timing the *next* command would prove nothing — that was already fast when
/// the hash ran inline. What matters is that the dispatch which *calls*
/// `create_account` comes back promptly, and that ordinary commands complete
/// before the auth result does.
#[test]
fn the_game_thread_keeps_serving_while_a_password_is_hashed() {
    let mut vm = RealVm::boot();

    let started = Instant::now();
    vm.eval("create_account(this_session(), 'alice', 'correct horse battery')")
        .unwrap();
    let submitted = started.elapsed();

    // Argon2 was measured at ~370 ms to hash on this machine. A dispatch that
    // returns in a fraction of that cannot have waited for it.
    assert!(
        submitted < Duration::from_millis(150),
        "the dispatch that called create_account took {submitted:?} — it is still \
         hashing on the game thread"
    );

    // And the VM answers ordinary work while the hash is in flight.
    for i in 0..3 {
        assert_eq!(vm.eval(&format!("return {i}")).unwrap(), i.to_string());
    }

    let reply = vm.next_auth_result();
    assert_eq!(reply.error, None, "the account should have been created");
    assert_eq!(reply.username.as_deref(), Some("alice"));
}

/// Creating an account and then logging into it, entirely through the hook.
#[test]
fn a_round_trip_through_on_auth_result_succeeds() {
    let mut vm = RealVm::boot();

    vm.eval("create_account(this_session(), 'bob', 'a good long password')")
        .unwrap();
    let created = vm.next_auth_result();
    assert_eq!(created.kind, "create_account");
    assert_eq!(created.username.as_deref(), Some("bob"));
    assert_eq!(created.error, None);

    vm.eval("authenticate(this_session(), 'bob', 'a good long password')")
        .unwrap();
    let logged_in = vm.next_auth_result();
    assert_eq!(logged_in.kind, "authenticate");
    assert_eq!(logged_in.username.as_deref(), Some("bob"));
    assert_eq!(logged_in.error, None);
}

/// A wrong password comes back through the same hook, and says nothing about
/// whether the account exists.
#[test]
fn a_bad_password_is_reported_through_the_hook() {
    let mut vm = RealVm::boot();

    vm.eval("create_account(this_session(), 'carol', 'a good long password')")
        .unwrap();
    vm.next_auth_result();

    vm.eval("authenticate(this_session(), 'carol', 'wrong')").unwrap();
    let bad_password = vm.next_auth_result();
    assert!(bad_password.username.is_none());
    let bad_password_msg = bad_password.error.unwrap();

    vm.eval("authenticate(this_session(), 'nobody_at_all', 'wrong')")
        .unwrap();
    let no_such_user = vm.next_auth_result();
    assert!(no_such_user.username.is_none());

    assert_eq!(
        bad_password_msg,
        no_such_user.error.unwrap(),
        "a wrong password and a missing account must be indistinguishable"
    );
}

/// Five consecutive failures lock the address out, and the refusal arrives on
/// the same hook rather than as a separate failure mode for the mudlib.
#[test]
fn repeated_failures_are_refused_without_hashing() {
    let mut vm = RealVm::boot();

    for _ in 0..5 {
        vm.eval("authenticate(this_session(), 'dave', 'wrong')").unwrap();
        assert!(vm.next_auth_result().error.is_some());
    }

    let started = Instant::now();
    vm.eval("authenticate(this_session(), 'dave', 'wrong')").unwrap();
    let refused = vm.next_auth_result();
    let elapsed = started.elapsed();

    let msg = refused.error.expect("a locked-out attempt must fail");
    assert!(
        msg.contains("Too many failed attempts"),
        "expected a lockout message, got {msg:?}"
    );
    assert!(
        elapsed < Duration::from_millis(100),
        "the refusal took {elapsed:?} — it should skip Argon2 entirely"
    );
}
