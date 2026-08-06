//! `LuaCommand::TimerFired` never set a session, so `get_current_session()` was
//! `None` during every daemon tick and `check_efun_permission` failed closed:
//! any gated efun called from a timer was denied. Nothing was connected to see
//! the error, so the failure was silent.
//!
//! The engine now declares an explicit identity for its own dispatches instead
//! of letting the answer fall out of an unset thread-local. These check both
//! halves of that: a tick may use gated efuns, and an ordinary player session
//! still may not.

mod common;

use std::collections::HashMap;

use common::RealVm;
use oxigeon::config::permissions_config::{DirPerms, PermissionConfig};

/// `write_file` under a directory that requires a permission — the shape of
/// `audit_d.lua`, which writes on a tick.
fn gated_writes() -> PermissionConfig {
    let mut directories = HashMap::new();
    directories.insert(
        "/mudlib/logs".to_string(),
        DirPerms { read: None, write: Some("admin".to_string()) },
    );
    PermissionConfig { directories, ..Default::default() }
}

fn gated_efun() -> PermissionConfig {
    let mut efuns = HashMap::new();
    // `journal_read` rather than `broadcast`: it is gated the same way but
    // sends nothing to the probe session, so the reply stream stays clean.
    efuns.insert("journal_read".to_string(), "admin".to_string());
    PermissionConfig { efuns, ..Default::default() }
}

/// The trap the review described: a daemon that writes on a tick.
#[test]
fn a_timer_tick_may_write_to_a_permission_gated_directory() {
    let mut vm = RealVm::boot_with_permissions(gated_writes());

    let wrote = vm.eval_on_timer("return tostring(write_file('logs/tick.txt', 'from a tick'))");
    assert_eq!(
        wrote.unwrap(),
        "true",
        "a daemon tick could not write to a gated directory — this is the silent \
         failure the explicit system identity exists to prevent"
    );

    assert_eq!(
        vm.eval("return read_file('logs/tick.txt')").unwrap(),
        "from a tick"
    );
}

/// A gated efun called on a tick is allowed.
#[test]
fn a_timer_tick_may_call_a_gated_efun() {
    let mut vm = RealVm::boot_with_permissions(gated_efun());
    let result = vm.eval_on_timer("journal_read(1) return 'called'");
    assert_eq!(
        result.unwrap(),
        "called",
        "a gated efun should be permitted for engine-internal dispatch"
    );
}

/// And the gate is still a gate. Widening it for the engine must not widen it
/// for a player session that has no permissions.
#[test]
fn a_player_session_still_cannot_call_a_gated_efun() {
    let mut vm = RealVm::boot_with_permissions(gated_efun());
    let denied = vm.eval("journal_read(1) return 'called'");
    assert!(
        denied.is_err(),
        "an unprivileged session must still be refused, got {denied:?}"
    );
    assert!(denied.err().contains("Permission denied"));
}

/// Same for the directory gate.
#[test]
fn a_player_session_still_cannot_write_to_a_gated_directory() {
    let mut vm = RealVm::boot_with_permissions(gated_writes());
    assert_eq!(
        vm.eval("return tostring(write_file('logs/player.txt', 'nope'))").unwrap(),
        "false",
        "an unprivileged session must still be refused"
    );
}

/// The identity is scoped to the dispatch, not sticky. A tick that grants
/// privilege must not leave it granted for the next player command.
#[test]
fn the_system_identity_does_not_leak_into_the_next_dispatch() {
    let mut vm = RealVm::boot_with_permissions(gated_efun());

    assert_eq!(vm.eval_on_timer("journal_read(1) return 'called'").unwrap(), "called");

    let after = vm.eval("journal_read(1) return 'called'");
    assert!(
        after.is_err(),
        "the tick's privilege leaked into the following player dispatch: {after:?}"
    );
}

/// A tick whose Lua raises must not leave the identity set either — the guard
/// has to restore it on the way out, not only on the happy path.
#[test]
fn an_erroring_tick_still_restores_the_previous_identity() {
    let mut vm = RealVm::boot_with_permissions(gated_efun());

    let boom = vm.eval_on_timer("error('deliberate failure inside a tick')");
    assert!(boom.is_err());

    let after = vm.eval("journal_read(1) return 'called'");
    assert!(
        after.is_err(),
        "privilege survived an erroring tick: {after:?}"
    );
}
