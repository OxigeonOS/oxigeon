//! D4 — a role change must take effect for a player who is already online.
//!
//! `has_permission` reads a per-session cache seeded once, at
//! `enter_game_session`. `refresh_permissions` repopulates it, and it had no
//! caller in `mudlib/` or `game/` and no test anywhere — so nothing had ever
//! established what actually reaches an online session and what does not.
//!
//! Asking the question turned up a real asymmetry rather than the suspected
//! total failure. `assign_role` and `revoke_role` already pushed into every
//! matching session's cache. `grant_permission` and `revoke_permission` — which
//! change what a role *contains*, and so change what everyone holding it may do
//! — did not. Two halves of one surface behaving two ways is worse than either
//! behaviour applied consistently: an admin who watched a role assignment land
//! immediately has every reason to expect editing the role to do the same.
//!
//! Both now resync, and `refresh_permissions` stays as the explicit escape
//! hatch for anything the automatic path cannot see.
//!
//! Through the engine's real VM per `CLAUDE.md`: the cache lives on the
//! `SessionHandler` the driver owns, and a test that built its own handler
//! would be asking about a different object than the one `has_permission` reads.

use crate::common::RealVm;
use oxigeon::domain::models::DieselAccountStore;

const PERM: &str = "dir.write.areas";

/// Put this VM's session into the playing state with a real account and
/// character, the way login does. Returns `(account_id, character_id)`.
///
/// `enter_game_session` and `create_character` go through the efuns, because
/// `enter_game_session` is what seeds the permission cache and seeding it is
/// the thing under test. The accounts are made through the store directly: the
/// `create_account` efun answers asynchronously through the *real* mudlib's
/// `on_auth_result`, which drives the login flow rather than replying to a
/// probe, so waiting on it here would wait forever for a message addressed to
/// somebody else.
///
/// `admin` picks which account the session enters as. Account 1 is the
/// superuser and bypasses every permission check, so a test about permissions
/// needs an ordinary account — and one about the bypass needs account 1. Both
/// are created either way, so the ids are stable whichever is asked for.
fn enter_game(vm: &mut RealVm, admin: bool) -> (i64, i64) {
    let sid = vm.session_id().to_string();
    let accounts = DieselAccountStore::new(vm.pool(), 6);

    let superuser = accounts
        .create("rbacroot", "a good long test password")
        .expect("create the superuser account");
    assert!(superuser.is_admin, "the first account should be the superuser");

    let ordinary = accounts
        .create("rbacuser", "a good long test password")
        .expect("create an ordinary account");
    assert!(!ordinary.is_admin, "the second account should not be an admin");

    let account_id = if admin { superuser.id } else { ordinary.id };

    let character_id: i64 = vm
        .eval(&format!("return create_character({account_id}, 'Rbactest').id"))
        .unwrap()
        .parse()
        .expect("character id");

    // Not wrapped in `tostring` — this returns nothing, and `tostring()` with
    // no argument raises.
    vm.eval(&format!(
        "enter_game_session('{sid}', {account_id}, {character_id}) return 'entered'"
    ))
    .unwrap();

    assert_eq!(
        vm.eval(&format!("return get_session('{sid}').state")).unwrap(),
        "playing",
        "the session did not reach the playing state"
    );

    (account_id, character_id)
}

/// Whether the session currently believes it holds the permission.
fn allowed(vm: &mut RealVm) -> bool {
    let sid = vm.session_id().to_string();
    vm.eval(&format!("return tostring(has_permission('{sid}', '{PERM}'))"))
        .unwrap()
        == "true"
}

/// Assigning a role to someone who is already playing reaches them without any
/// further call. This was already true; it is asserted so it stays true.
#[test]
fn assigning_a_role_reaches_an_online_session_immediately() {
    let mut vm = RealVm::boot_fixture_with_probe();
    let (_account_id, character_id) = enter_game(&mut vm, false);

    assert!(!allowed(&mut vm), "a fresh character should hold no permissions");

    vm.eval("create_role('builder')").unwrap();
    vm.eval(&format!("grant_permission('builder', '{PERM}')")).unwrap();
    assert_eq!(
        vm.eval("return tostring(#get_permissions('builder') > 0)").unwrap(),
        "true",
        "the role should carry the permission in the store"
    );

    vm.eval(&format!("assign_role({character_id}, 'builder')")).unwrap();
    assert!(
        allowed(&mut vm),
        "assign_role should have pushed the new permissions into the live session"
    );

    vm.eval(&format!("revoke_role({character_id}, 'builder')")).unwrap();
    assert!(
        !allowed(&mut vm),
        "revoke_role should have taken them away again — a permission that \
         outlives its revocation is a security problem, not an inconvenience"
    );
}

/// **The gap this file found.** Editing a role the player already holds changes
/// what they may do, and used to reach nobody who was already online.
#[test]
fn editing_a_role_reaches_the_sessions_that_hold_it() {
    let mut vm = RealVm::boot_fixture_with_probe();
    let (_account_id, character_id) = enter_game(&mut vm, false);

    // A role of this test's own, because the game layer declares `builder` with
    // permissions already on it — asserting against a role somebody else owns
    // would make this test a test of `setup_roles.lua`.
    //
    // Hold it first, while it grants nothing. That ordering is what exposes the
    // bug: `assign_role` cannot push a permission that does not exist yet, so
    // the only thing that can is the grant itself.
    vm.eval("create_role('probe_role')").unwrap();
    vm.eval(&format!("assign_role({character_id}, 'probe_role')")).unwrap();
    assert!(!allowed(&mut vm), "an empty role should grant nothing");

    vm.eval(&format!("grant_permission('probe_role', '{PERM}')")).unwrap();
    assert!(
        allowed(&mut vm),
        "granting a permission to a role the player already holds did not reach \
         their session — this is the half of the RBAC surface that did not resync"
    );

    vm.eval(&format!("revoke_permission('probe_role', '{PERM}')")).unwrap();
    assert!(
        !allowed(&mut vm),
        "revoking it from the role should reach them the same way"
    );
}

/// `refresh_permissions` is the explicit escape hatch, and has to work even
/// when nothing has changed — an idempotent refresh is what makes it safe to
/// call from a command without reasoning about whether it is needed.
#[test]
fn an_explicit_refresh_rebuilds_the_cache_from_the_store() {
    let mut vm = RealVm::boot_fixture_with_probe();
    let (_account_id, character_id) = enter_game(&mut vm, false);
    let sid = vm.session_id().to_string();

    vm.eval("create_role('builder')").unwrap();
    vm.eval(&format!("grant_permission('builder', '{PERM}')")).unwrap();
    vm.eval(&format!("assign_role({character_id}, 'builder')")).unwrap();
    assert!(allowed(&mut vm));

    // Twice, to pin idempotence: a refresh clears the cache before repopulating
    // it, and a bug there would show up as the second call dropping everything.
    for _ in 0..2 {
        assert_eq!(
            vm.eval(&format!("return tostring(refresh_permissions('{sid}'))")).unwrap(),
            "true",
            "refresh_permissions should report that it repopulated the cache"
        );
        assert!(allowed(&mut vm), "a refresh dropped a permission that is still granted");
    }
}

/// A session that is not playing has no character to look permissions up for,
/// so this reports failure rather than silently leaving a stale cache in place.
#[test]
fn refreshing_a_session_that_is_not_playing_reports_failure() {
    let mut vm = RealVm::boot_fixture_with_probe();
    let sid = vm.session_id().to_string();

    assert_eq!(
        vm.eval(&format!("return tostring(refresh_permissions('{sid}'))")).unwrap(),
        "false",
        "a connected-but-not-playing session has no character to refresh from"
    );
    assert_eq!(
        vm.eval("return tostring(refresh_permissions('not-a-session-id'))").unwrap(),
        "false"
    );
}

/// The superuser bypass is not a granted permission and must survive every
/// resync — otherwise editing an unrelated role would be a way to demote
/// account 1.
#[test]
fn a_resync_preserves_the_admin_flag() {
    let mut vm = RealVm::boot_fixture_with_probe();
    let (account_id, character_id) = enter_game(&mut vm, true);
    let sid = vm.session_id().to_string();

    assert_eq!(account_id, 1, "expected the superuser to be account 1");
    assert_eq!(
        vm.eval(&format!("return tostring(has_permission('{sid}', 'anything.at.all'))")).unwrap(),
        "true",
        "account 1 should bypass permission checks"
    );

    // Every path that rewrites the cache, in turn.
    vm.eval("create_role('builder')").unwrap();
    vm.eval(&format!("assign_role({character_id}, 'builder')")).unwrap();
    vm.eval(&format!("grant_permission('builder', '{PERM}')")).unwrap();
    vm.eval(&format!("revoke_permission('probe_role', '{PERM}')")).unwrap();
    vm.eval(&format!("revoke_role({character_id}, 'builder')")).unwrap();
    vm.eval(&format!("refresh_permissions('{sid}')")).unwrap();

    assert_eq!(
        vm.eval(&format!("return tostring(has_permission('{sid}', 'anything.at.all'))")).unwrap(),
        "true",
        "a resync cleared the superuser bypass"
    );
}
