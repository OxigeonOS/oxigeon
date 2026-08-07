//! Integration tests for character store CRUD and per-account limits.
//! Uses a temporary file-based SQLite database.

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

fn create_test_account(pool: &oxigeon::domain::db::connection::AnyPool) -> i64 {
    let store = oxigeon::domain::models::DieselAccountStore::new(pool.clone(), 6);
    let account = store.create("chartest", "password123").unwrap();
    account.id
}

#[test]
fn test_create_character() {
    let (pool, _dir) = make_test_db();
    let account_id = create_test_account(&pool);
    let store = oxigeon::domain::models::DieselCharacterStore::new(pool, 5);
    let character = store.create(account_id, "Aldric").unwrap();
    assert_eq!(character.name, "Aldric");
    assert_eq!(character.account_id, account_id);
}

#[test]
fn test_find_characters_by_account() {
    let (pool, _dir) = make_test_db();
    let account_id = create_test_account(&pool);
    let store = oxigeon::domain::models::DieselCharacterStore::new(pool, 5);
    store.create(account_id, "Aldric").unwrap();
    store.create(account_id, "Brenna").unwrap();
    let chars = store.find_by_account(account_id).unwrap();
    assert_eq!(chars.len(), 2);
}

#[test]
fn test_max_characters_per_account() {
    let (pool, _dir) = make_test_db();
    let account_id = create_test_account(&pool);
    let store = oxigeon::domain::models::DieselCharacterStore::new(pool, 2);
    store.create(account_id, "CharOne").unwrap();
    store.create(account_id, "CharTwo").unwrap();
    // Third should fail
    let result = store.create(account_id, "CharThree");
    assert!(result.is_err(), "Expected error when exceeding character limit");
}

#[test]
fn test_find_character_by_id() {
    let (pool, _dir) = make_test_db();
    let account_id = create_test_account(&pool);
    let store = oxigeon::domain::models::DieselCharacterStore::new(pool, 5);
    let created = store.create(account_id, "Findme").unwrap();
    let found = store.find_by_id(created.id).unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Findme");
}

#[test]
fn test_find_nonexistent_character() {
    let (pool, _dir) = make_test_db();
    let store = oxigeon::domain::models::DieselCharacterStore::new(pool, 5);
    let found = store.find_by_id(99999).unwrap();
    assert!(found.is_none());
}

#[test]
fn test_character_name_unique() {
    let (pool, _dir) = make_test_db();
    let account_id = create_test_account(&pool);
    let store = oxigeon::domain::models::DieselCharacterStore::new(pool, 5);
    store.create(account_id, "Unique").unwrap();
    let result = store.create(account_id, "Unique");
    assert!(result.is_err(), "Duplicate character names should fail");
}
