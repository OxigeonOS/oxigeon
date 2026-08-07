//! Unit tests for PermissionConfig parsing and directory permission lookup.
//! These tests are pure Rust — no database required.

use oxigeon::config::PermissionConfig;

// ─── Default / empty config ──────────────────────────────────────────────────

#[test]
fn test_empty_config_allows_everything() {
    let config = PermissionConfig::default();
    // No restrictions on any path or operation
    assert!(config.dir_permission("/game/data/test.txt", "read").is_none());
    assert!(config.dir_permission("/game/data/test.txt", "write").is_none());
    assert!(config.dir_permission("/mudlib/admin/secret.lua", "read").is_none());
    assert!(config.efuns.get("reload").is_none());
    assert!(config.efuns.get("broadcast").is_none());
}

// ─── TOML parsing ────────────────────────────────────────────────────────────

#[test]
fn test_parse_valid_toml() {
    let toml = r#"
[efuns]
reload    = "efun.reload"
broadcast = "efun.broadcast"

[directories]
"/mudlib/admin" = { read = "dir.read.admin", write = "dir.write.admin" }
"/game/data"  = { write = "dir.write.data" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    assert_eq!(config.efuns.get("reload").map(String::as_str), Some("efun.reload"));
    assert_eq!(config.efuns.get("broadcast").map(String::as_str), Some("efun.broadcast"));

    let admin = config.directories.get("/mudlib/admin").unwrap();
    assert_eq!(admin.read.as_deref(), Some("dir.read.admin"));
    assert_eq!(admin.write.as_deref(), Some("dir.write.admin"));

    let data = config.directories.get("/game/data").unwrap();
    assert!(data.read.is_none(), "read should be unrestricted");
    assert_eq!(data.write.as_deref(), Some("dir.write.data"));
}

#[test]
fn test_parse_efun_only_toml() {
    let toml = r#"
[efuns]
delete_file = "efun.delete_file"
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.efuns.get("delete_file").map(String::as_str), Some("efun.delete_file"));
    assert!(config.directories.is_empty());
}

#[test]
fn test_parse_directory_only_toml() {
    let toml = r#"
[directories]
"/mudlib/cmds" = { write = "dir.write.cmds" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    assert!(config.efuns.is_empty());
    let cmds = config.directories.get("/mudlib/cmds").unwrap();
    assert!(cmds.read.is_none());
    assert_eq!(cmds.write.as_deref(), Some("dir.write.cmds"));
}

// ─── dir_permission() — no match ─────────────────────────────────────────────

#[test]
fn test_dir_permission_unconfigured_path_allows() {
    let toml = r#"
[directories]
"/mudlib/admin" = { write = "dir.write.admin" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    // /data is not in config at all
    assert!(config.dir_permission("/game/data/foo.txt", "write").is_none());
    assert!(config.dir_permission("/game/data/foo.txt", "read").is_none());
}

#[test]
fn test_dir_permission_root_path_no_match() {
    let config = PermissionConfig::default();
    assert!(config.dir_permission("/", "read").is_none());
    assert!(config.dir_permission("/", "write").is_none());
}

// ─── dir_permission() — prefix matching ──────────────────────────────────────

#[test]
fn test_dir_permission_direct_prefix_match() {
    let toml = r#"
[directories]
"/mudlib/admin" = { read = "dir.read.admin", write = "dir.write.admin" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    assert_eq!(
        config.dir_permission("/mudlib/admin/config.lua", "write").map(String::as_str),
        Some("dir.write.admin")
    );
    assert_eq!(
        config.dir_permission("/mudlib/admin/subdir/file.lua", "read").map(String::as_str),
        Some("dir.read.admin")
    );
}

#[test]
fn test_dir_permission_longest_prefix_wins() {
    let toml = r#"
[directories]
"/game/data"       = { write = "dir.write.data" }
"/game/data/admin" = { write = "dir.write.data.admin" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    // Should match /game/data/admin (longer prefix)
    assert_eq!(
        config.dir_permission("/game/data/admin/secret.lua", "write").map(String::as_str),
        Some("dir.write.data.admin")
    );
    // Should match /game/data (shorter prefix)
    assert_eq!(
        config.dir_permission("/game/data/other.lua", "write").map(String::as_str),
        Some("dir.write.data")
    );
}

#[test]
fn test_dir_permission_read_open_when_only_write_configured() {
    let toml = r#"
[directories]
"/game/data" = { write = "dir.write.data" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    // Read is open (None = no restriction)
    assert!(config.dir_permission("/game/data/foo.txt", "read").is_none());
    // Write is restricted
    assert!(config.dir_permission("/game/data/foo.txt", "write").is_some());
}

#[test]
fn test_dir_permission_write_open_when_only_read_configured() {
    let toml = r#"
[directories]
"/mudlib/logs" = { read = "dir.read.logs" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    assert!(config.dir_permission("/mudlib/logs/server.log", "read").is_some());
    assert!(config.dir_permission("/mudlib/logs/server.log", "write").is_none());
}

#[test]
fn test_dir_permission_unknown_op_returns_none() {
    let toml = r#"
[directories]
"/mudlib/admin" = { read = "dir.read.admin", write = "dir.write.admin" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    // "execute" op is not defined — always returns None
    assert!(config.dir_permission("/mudlib/admin/script.lua", "execute").is_none());
}

// ─── efun gating ─────────────────────────────────────────────────────────────

#[test]
fn test_efun_not_in_config_returns_none() {
    let config = PermissionConfig::default();
    assert!(config.efuns.get("get_session").is_none());
    assert!(config.efuns.get("send").is_none());
}

#[test]
fn test_efun_in_config_returns_permission_string() {
    let toml = r#"
[efuns]
reload      = "efun.reload"
delete_file = "efun.delete_file"
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.efuns.get("reload").map(String::as_str), Some("efun.reload"));
    assert_eq!(config.efuns.get("delete_file").map(String::as_str), Some("efun.delete_file"));
    assert!(config.efuns.get("send").is_none());
}

// ─── the two-root migration ──────────────────────────────────────────────────

/// A directory key that names no root is refused rather than silently applied.
///
/// The file efuns are jailed to two trees, so `/areas` has no single answer: it
/// could mean the mudlib's, the game's, or both, and those are different trust
/// levels. Guessing would reproduce the failure `permissions.toml` already
/// documents once — a boundary everybody believed in that was not there.
#[test]
fn a_directory_key_naming_no_root_is_dropped_and_reported() {
    let toml = r#"
[directories]
"/areas" = { write = "dir.write.areas" }
"/game/areas" = { write = "dir.write.game.areas" }
"#;
    let mut config: PermissionConfig = toml::from_str(toml).unwrap();
    let dropped = config.take_invalid_directory_keys();

    assert_eq!(dropped, 1);
    assert_eq!(config.invalid_directory_keys, vec!["/areas".to_string()]);

    // Not applied to either root, and not to its literal spelling.
    assert!(config.dir_permission("/mudlib/areas/x.lua", "write").is_none());
    assert!(config.dir_permission("/areas/x.lua", "write").is_none());

    // The well-formed rule beside it is untouched.
    assert_eq!(
        config.dir_permission("/game/areas/crypt/rooms.lua", "write").map(String::as_str),
        Some("dir.write.game.areas")
    );
}

/// The repository's own config parses cleanly, with every rule in effect.
///
/// Against the shipped file rather than a fixture, because a fixture would pass
/// whether or not the real one had regressed — which is the entire reason
/// `tests/staff.rs` boots with this file instead of the harness default.
#[test]
fn the_shipped_config_has_no_rules_that_protect_nothing() {
    let config = PermissionConfig::load_from_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config/permissions.toml")
            .as_path(),
    );

    assert!(
        config.invalid_directory_keys.is_empty(),
        "these rules name no root and are not in effect: {:?}",
        config.invalid_directory_keys
    );
    assert!(
        !config.directories.is_empty(),
        "the shipped config should have directory rules — did it fail to parse?"
    );
    assert_eq!(
        config.dir_permission("/game/areas/crypt/rooms.lua", "write").map(String::as_str),
        Some("dir.write.game.areas"),
        "the tree OLC writes to is ungated"
    );
}
