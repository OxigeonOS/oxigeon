//! `LuaCommand::Shutdown` used to break the engine loop without telling Lua
//! anything, and nothing on the Ctrl+C path joined the Lua thread — `Drop for
//! ScriptEngine` only sent the command and returned. So a clean restart asked
//! the mudlib to save nothing, and would not have waited if it had.
//!
//! `CHARACTER_D` is a write-back cache flushed by the autosave ticker, so that
//! silently discarded up to `game.autosave_seconds` of every online player's
//! progress on every clean restart.
//!
//! These tests go through the real engine — the dispatch, the identity it runs
//! under, the bound on the wait — and the last one goes through the real
//! mudlib, so it fails if `mudlib/init.lua` stops actually saving.

mod common;

use std::collections::HashMap;
use std::time::Duration;

use common::{Probe, RealVm};
use oxigeon::config::permissions_config::{DirPerms, PermissionConfig};
use oxigeon::domain::models::DieselCharacterStore;

/// Long enough that a healthy mudlib always finishes, short enough that a
/// broken one does not stall the suite.
const GENEROUS: Duration = Duration::from_secs(10);

fn gated_efun() -> PermissionConfig {
    let mut efuns = HashMap::new();
    efuns.insert("journal_read".to_string(), "admin".to_string());
    PermissionConfig { efuns, ..Default::default() }
}

fn gated_writes() -> PermissionConfig {
    let mut directories = HashMap::new();
    directories.insert(
        "/mudlib/logs".to_string(),
        DirPerms { read: None, write: Some("admin".to_string()) },
    );
    PermissionConfig { directories, ..Default::default() }
}

/// The bug itself: the mudlib is never told the server is going away.
#[test]
fn a_clean_shutdown_dispatches_on_shutdown() {
    let mut vm = RealVm::boot();
    let sid = vm.session_id().to_string();
    vm.eval(&format!("_shutdown_session = '{sid}' return 'armed'")).unwrap();

    assert!(vm.shutdown_within(GENEROUS), "the Lua thread did not finish");

    assert_eq!(
        vm.next_buffered_reply(),
        Some(Probe::Ok("ran".to_string())),
        "on_shutdown was never dispatched — a clean restart saves nothing"
    );
}

/// A shutdown has no player behind it, exactly like a timer tick. If it
/// dispatched with no identity, every gated efun the flush touched would be
/// refused — and the refusals would go to a server with nobody left on it.
#[test]
fn the_shutdown_dispatch_runs_as_the_engine() {
    let mut vm = RealVm::boot_with_permissions(gated_efun());
    let sid = vm.session_id().to_string();
    vm.eval(&format!(
        "_shutdown_session = '{sid}' _shutdown_source = [[journal_read(1) return 'called']] return 'armed'"
    ))
    .unwrap();

    assert!(vm.shutdown_within(GENEROUS));

    assert_eq!(
        vm.next_buffered_reply(),
        Some(Probe::Ok("called".to_string())),
        "a gated efun was refused during shutdown; the flush cannot do its job"
    );
}

/// The same question for the directory gate — `audit_d` and `journal_d` write
/// under `logs/`, and a shutdown flush is the last thing that will.
#[test]
fn the_shutdown_dispatch_may_write_to_a_gated_directory() {
    let mut vm = RealVm::boot_with_permissions(gated_writes());
    let sid = vm.session_id().to_string();
    vm.eval(&format!(
        "_shutdown_session = '{sid}' _shutdown_source = [[return tostring(write_file('logs/bye.txt', 'flushed'))]] return 'armed'"
    ))
    .unwrap();

    assert!(vm.shutdown_within(GENEROUS));

    assert_eq!(vm.next_buffered_reply(), Some(Probe::Ok("true".to_string())));
}

/// Waiting is the point, but an unbounded wait would hand any mudlib the power
/// to hang the process. The bound has to hold even when `on_shutdown` does not
/// return.
#[test]
fn a_wedged_on_shutdown_does_not_hang_the_server() {
    let mut vm = RealVm::boot();
    // Spins for a second or two — long enough to outlast the deadline below by
    // an order of magnitude, short enough that the thread is gone well before
    // the test binary is.
    vm.eval("_shutdown_source = [[local t = os_time() while os_time() - t < 2 do end return 'woke']] return 'armed'")
        .unwrap();

    let started = std::time::Instant::now();
    let finished = vm.shutdown_within(Duration::from_millis(150));

    assert!(!finished, "a wedged on_shutdown should report that it did not finish");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the wait was not bounded: {:?}",
        started.elapsed()
    );
}

/// End to end, through the mudlib the server actually ships.
///
/// `pagesize` writes to `player.custom`, which is in `SAVE_FIELDS` but only
/// reaches the database on an autosave tick, a disconnect, or a shutdown.
/// Autosave is off in the harness and the session never disconnects, so the
/// value is on disk afterwards only if `on_shutdown` saved it.
#[test]
fn a_clean_shutdown_saves_character_data_through_the_real_mudlib() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let out = vm.command("pagesize 17");
    assert!(out.contains("Page length set to 17"), "pagesize did not take: {out:?}");

    let store = DieselCharacterStore::new(vm.pool(), 5);
    let char_id = store
        .find_by_account(1)
        .expect("characters for the test account")
        .first()
        .expect("the login flow created a character")
        .id;

    let before = store.load_data(char_id).unwrap().unwrap_or_default();
    assert!(
        !before.contains("page_length"),
        "the change reached the database without a save — this test can no longer \
         tell whether the shutdown flushed anything: {before:?}"
    );

    assert!(vm.shutdown_within(GENEROUS), "the mudlib did not finish shutting down");

    let after = store.load_data(char_id).unwrap().unwrap_or_default();
    assert!(
        after.contains("\"page_length\":17"),
        "a clean shutdown did not save the character; the database still holds {after:?}"
    );
}
