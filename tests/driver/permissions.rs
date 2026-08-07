//! Integration tests for the RBAC permission system.
//! Tests the DieselRoleStore, SessionHandler permission cache, and Lua permission efuns.

use std::sync::{Arc, RwLock};
use tempfile::TempDir;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use oxigeon::domain::models::{DieselAccountStore, DieselCharacterStore};
use oxigeon::domain::models::role::DieselRoleStore;
use oxigeon::domain::db::connection::AnyPool;
use oxigeon::config::{DatabaseConfig, DatabaseBackend};
use oxigeon::core::session::{SessionHandler, Session, SessionOutput, SessionId};
use oxigeon::config::MultisessionMode;
use tokio::sync::mpsc;
use std::net::SocketAddr;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

fn make_test_db() -> (AnyPool, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let config = DatabaseConfig {
        backend: DatabaseBackend::Sqlite,
        url: db_path.to_string_lossy().to_string(),
        pool_size: 1,
    };
    let pool = AnyPool::new(&config).unwrap();
    {
        let mut conn = pool.get_sqlite().unwrap();
        conn.run_pending_migrations(MIGRATIONS).unwrap();
    }
    (pool, dir)
}

fn make_session_playing(
    sh: &Arc<RwLock<SessionHandler>>,
    account_id: i64,
    character_id: i64,
    perms: Vec<String>,
    is_admin: bool,
) -> SessionId {
    let (tx, _rx) = mpsc::channel::<SessionOutput>(16);
    let addr: SocketAddr = "127.0.0.1:9999".parse().unwrap();
    let session = Session::new("telnet".to_string(), addr, tx);
    let sid = session.id;
    sh.write().unwrap().connect(session).unwrap();
    sh.write().unwrap().enter_game(&sid, account_id, character_id, perms, is_admin).unwrap();
    sid
}

// ─── Role CRUD ─────────────────────────────────────────────────────────────

#[test]
fn test_create_and_list_roles() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);

    store.create_role("admin").unwrap();
    store.create_role("builder").unwrap();
    store.create_role("player").unwrap();

    let roles = store.list_roles().unwrap();
    assert_eq!(roles.len(), 3);
    let names: Vec<_> = roles.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"admin"));
    assert!(names.contains(&"builder"));
    assert!(names.contains(&"player"));
}

#[test]
fn test_role_name_must_be_unique() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);

    store.create_role("admin").unwrap();
    let result = store.create_role("admin");
    assert!(result.is_err(), "Duplicate role name should fail");
}

#[test]
fn test_delete_role_removes_it() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);

    let role = store.create_role("temp").unwrap();
    assert_eq!(store.list_roles().unwrap().len(), 1);
    store.delete_role(role.id).unwrap();
    assert_eq!(store.list_roles().unwrap().len(), 0);
}

// ─── Permissions ────────────────────────────────────────────────────────────

#[test]
fn test_grant_and_revoke_permission() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);

    let role = store.create_role("admin").unwrap();
    store.grant_permission(role.id, "efun.reload").unwrap();
    store.grant_permission(role.id, "efun.broadcast").unwrap();

    let perms = store.get_permissions_for_role(role.id).unwrap();
    assert!(perms.contains(&"efun.reload".to_string()));
    assert!(perms.contains(&"efun.broadcast".to_string()));

    store.revoke_permission(role.id, "efun.reload").unwrap();
    let perms = store.get_permissions_for_role(role.id).unwrap();
    assert!(!perms.contains(&"efun.reload".to_string()));
    assert!(perms.contains(&"efun.broadcast".to_string()));
}

#[test]
fn test_grant_permission_idempotent() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);

    let role = store.create_role("admin").unwrap();
    store.grant_permission(role.id, "efun.reload").unwrap();
    store.grant_permission(role.id, "efun.reload").unwrap(); // second grant should not error
    let perms = store.get_permissions_for_role(role.id).unwrap();
    assert_eq!(perms.len(), 1);
}

// ─── Character ↔ Role ───────────────────────────────────────────────────────

#[test]
fn test_assign_and_revoke_character_role() {
    let (pool, _dir) = make_test_db();
    let account_store = DieselAccountStore::new(pool.clone(), 6);
    let char_store = DieselCharacterStore::new(pool.clone(), 5);
    let role_store = DieselRoleStore::new(pool);

    let account = account_store.create("alice", "password1").unwrap();
    let character = char_store.create(account.id, "Alice").unwrap();
    let role = role_store.create_role("builder").unwrap();

    role_store.assign_role(character.id, role.id).unwrap();
    let roles = role_store.get_roles_for_character(character.id).unwrap();
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "builder");

    role_store.revoke_role(character.id, role.id).unwrap();
    let roles = role_store.get_roles_for_character(character.id).unwrap();
    assert_eq!(roles.len(), 0);
}

#[test]
fn test_get_permissions_for_character_union() {
    let (pool, _dir) = make_test_db();
    let account_store = DieselAccountStore::new(pool.clone(), 6);
    let char_store = DieselCharacterStore::new(pool.clone(), 5);
    let store = DieselRoleStore::new(pool);

    let account = account_store.create("bob", "password2").unwrap();
    let character = char_store.create(account.id, "Bob").unwrap();

    let role1 = store.create_role("builder").unwrap();
    let role2 = store.create_role("moderator").unwrap();

    store.grant_permission(role1.id, "dir.write.areas").unwrap();
    store.grant_permission(role2.id, "efun.broadcast").unwrap();
    store.grant_permission(role2.id, "efun.disconnect").unwrap();

    store.assign_role(character.id, role1.id).unwrap();
    store.assign_role(character.id, role2.id).unwrap();

    let perms = store.get_permissions_for_character(character.id).unwrap();
    assert!(perms.contains(&"dir.write.areas".to_string()));
    assert!(perms.contains(&"efun.broadcast".to_string()));
    assert!(perms.contains(&"efun.disconnect".to_string()));
    assert_eq!(perms.len(), 3);
}

#[test]
fn test_cascade_delete_role_removes_character_roles() {
    let (pool, _dir) = make_test_db();
    let account_store = DieselAccountStore::new(pool.clone(), 6);
    let char_store = DieselCharacterStore::new(pool.clone(), 5);
    let store = DieselRoleStore::new(pool);

    let account = account_store.create("carol", "password3").unwrap();
    let character = char_store.create(account.id, "Carol").unwrap();
    let role = store.create_role("temp").unwrap();
    store.grant_permission(role.id, "efun.test").unwrap();
    store.assign_role(character.id, role.id).unwrap();

    // Delete role
    store.delete_role(role.id).unwrap();

    // Character should have no roles or permissions
    let roles = store.get_roles_for_character(character.id).unwrap();
    assert_eq!(roles.len(), 0);
    let perms = store.get_permissions_for_character(character.id).unwrap();
    assert_eq!(perms.len(), 0);
}

// ─── Session Permission Cache ────────────────────────────────────────────────

#[test]
fn test_session_no_permission_returns_false() {
    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));
    let sid = make_session_playing(&sh, 1, 1, vec![], false);
    assert!(!sh.read().unwrap().has_permission(&sid, "efun.reload"));
}

#[test]
fn test_session_has_permission_returns_true() {
    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));
    let sid = make_session_playing(
        &sh, 1, 1,
        vec!["efun.reload".to_string(), "efun.broadcast".to_string()],
        false
    );
    assert!(sh.read().unwrap().has_permission(&sid, "efun.reload"));
    assert!(sh.read().unwrap().has_permission(&sid, "efun.broadcast"));
    assert!(!sh.read().unwrap().has_permission(&sid, "efun.delete"));
}

#[test]
fn test_superuser_bypasses_all_permissions() {
    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));
    let sid = make_session_playing(&sh, 1, 1, vec![], true); // is_admin=true
    // Superuser returns true for ANY perm string
    assert!(sh.read().unwrap().has_permission(&sid, "efun.reload"));
    assert!(sh.read().unwrap().has_permission(&sid, "some.made.up.perm"));
    assert!(sh.read().unwrap().has_permission(&sid, "dir.write.admin"));
}

#[test]
fn test_set_permissions_refreshes_cache() {
    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));
    let sid = make_session_playing(&sh, 1, 1, vec![], false);
    assert!(!sh.read().unwrap().has_permission(&sid, "efun.reload"));

    // Refresh with a new permission set
    sh.write().unwrap().set_permissions(
        &sid,
        vec!["efun.reload".to_string()],
        false
    ).unwrap();

    assert!(sh.read().unwrap().has_permission(&sid, "efun.reload"));
    assert!(!sh.read().unwrap().has_permission(&sid, "efun.broadcast"));
}

#[test]
fn test_superuser_set_via_set_permissions() {
    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));
    let sid = make_session_playing(&sh, 1, 1, vec![], false);
    // Initially not superuser
    assert!(!sh.read().unwrap().has_permission(&sid, "anything"));

    // Upgrade to superuser
    sh.write().unwrap().set_permissions(&sid, vec![], true).unwrap();
    assert!(sh.read().unwrap().has_permission(&sid, "anything"));
    assert!(sh.read().unwrap().has_permission(&sid, "efun.reload"));

    // Downgrade from superuser
    sh.write().unwrap().set_permissions(&sid, vec!["efun.chat".to_string()], false).unwrap();
    assert!(!sh.read().unwrap().has_permission(&sid, "efun.reload"));
    assert!(sh.read().unwrap().has_permission(&sid, "efun.chat"));
}

#[test]
fn test_has_permission_unknown_session_returns_false() {
    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));
    // Use a random session id that was never registered
    let fake_sid: SessionId = "00000000-0000-0000-0000-000000000001".parse().unwrap();
    assert!(!sh.read().unwrap().has_permission(&fake_sid, "anything"));
}

// ─── find_role_by_name ──────────────────────────────────────────────────────

#[test]
fn test_find_role_by_name_exists() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);
    store.create_role("moderator").unwrap();
    let found = store.find_role_by_name("moderator").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "moderator");
}

#[test]
fn test_find_role_by_name_missing() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);
    let found = store.find_role_by_name("nonexistent").unwrap();
    assert!(found.is_none());
}

#[test]
fn test_find_role_by_id() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);
    let role = store.create_role("tester").unwrap();
    let found = store.find_role_by_id(role.id).unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "tester");
    let missing = store.find_role_by_id(9999).unwrap();
    assert!(missing.is_none());
}

// ─── Empty-case tests ────────────────────────────────────────────────────────

#[test]
fn test_get_permissions_for_empty_role() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);
    let role = store.create_role("empty").unwrap();
    let perms = store.get_permissions_for_role(role.id).unwrap();
    assert!(perms.is_empty());
}

#[test]
fn test_get_roles_for_character_with_no_roles() {
    let (pool, _dir) = make_test_db();
    let account_store = DieselAccountStore::new(pool.clone(), 6);
    let char_store = DieselCharacterStore::new(pool.clone(), 5);
    let role_store = DieselRoleStore::new(pool);

    let account = account_store.create("dave", "password4").unwrap();
    let character = char_store.create(account.id, "Dave").unwrap();

    let roles = role_store.get_roles_for_character(character.id).unwrap();
    assert!(roles.is_empty());
    let perms = role_store.get_permissions_for_character(character.id).unwrap();
    assert!(perms.is_empty());
}

#[test]
fn test_list_roles_empty() {
    let (pool, _dir) = make_test_db();
    let store = DieselRoleStore::new(pool);
    let roles = store.list_roles().unwrap();
    assert!(roles.is_empty());
}
