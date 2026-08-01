//! Integration tests for account store CRUD and authentication.
//! Uses a temporary file-based SQLite database (in-memory SQLite doesn't
//! share state across r2d2 pool connections).

use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

fn make_test_db() -> (oxigeon::domain::db::connection::AnyPool, tempfile::TempDir) {
    use oxigeon::config::driver_config::{DatabaseConfig, DatabaseBackend};
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let config = DatabaseConfig {
        backend: DatabaseBackend::Sqlite,
        url: db_path.to_string_lossy().to_string(),
        pool_size: 1,
    };
    let pool = oxigeon::domain::db::connection::AnyPool::new(&config).unwrap();
    let mut conn = pool.get_sqlite().unwrap();
    conn.run_pending_migrations(MIGRATIONS).unwrap();
    (pool, dir)
}

#[test]
fn test_create_account() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselAccountStore::new(pool, 6);
    let account = store.create("testuser", "password123").unwrap();
    assert_eq!(account.username, "testuser");
    assert!(!account.password_hash.is_empty());
    assert!(account.is_admin); // First account is auto-promoted to admin
}

#[test]
fn test_authenticate_correct_password() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselAccountStore::new(pool, 6);
    store.create("alice", "correct_horse").unwrap();
    let account = store.authenticate("alice", "correct_horse").unwrap();
    assert_eq!(account.username, "alice");
}

#[test]
fn test_authenticate_wrong_password() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselAccountStore::new(pool, 6);
    store.create("bob", "rightpassword").unwrap();
    let result = store.authenticate("bob", "wrongpassword");
    assert!(result.is_err());
}

#[test]
fn test_authenticate_nonexistent_user() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselAccountStore::new(pool, 6);
    let result = store.authenticate("ghost", "anypass");
    assert!(result.is_err());
}

#[test]
fn test_create_duplicate_account_fails() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselAccountStore::new(pool, 6);
    store.create("dupeuser", "password1").unwrap();
    let result = store.create("dupeuser", "password2");
    assert!(result.is_err());
}

#[test]
fn test_password_too_short() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselAccountStore::new(pool, 8);
    let result = store.create("shortpw", "abc");
    assert!(result.is_err());
}

#[test]
fn test_find_account_by_id() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselAccountStore::new(pool, 6);
    let account = store.create("findme", "password123").unwrap();
    let found = store.find_by_id(account.id).unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().username, "findme");
}

#[test]
fn test_find_account_by_name() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselAccountStore::new(pool, 6);
    store.create("named", "password123").unwrap();
    let found = store.find_by_name("named").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().username, "named");
}

#[test]
fn test_find_nonexistent_account() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselAccountStore::new(pool, 6);
    let found = store.find_by_name("nobody").unwrap();
    assert!(found.is_none());
}
