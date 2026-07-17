//! Unit tests for PermissionConfig parsing and directory permission lookup.
//! These tests are pure Rust — no database required.

use oxigeon::config::PermissionConfig;

// ─── Default / empty config ──────────────────────────────────────────────────

#[test]
fn test_empty_config_allows_everything() {
    let config = PermissionConfig::default();
    // No restrictions on any path or operation
    assert!(config.dir_permission("/data/test.txt", "read").is_none());
    assert!(config.dir_permission("/data/test.txt", "write").is_none());
    assert!(config.dir_permission("/admin/secret.lua", "read").is_none());
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
"/admin" = { read = "dir.read.admin", write = "dir.write.admin" }
"/data"  = { write = "dir.write.data" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    assert_eq!(config.efuns.get("reload").map(String::as_str), Some("efun.reload"));
    assert_eq!(config.efuns.get("broadcast").map(String::as_str), Some("efun.broadcast"));

    let admin = config.directories.get("/admin").unwrap();
    assert_eq!(admin.read.as_deref(), Some("dir.read.admin"));
    assert_eq!(admin.write.as_deref(), Some("dir.write.admin"));

    let data = config.directories.get("/data").unwrap();
    assert!(data.read.is_none(), "read should be unrestricted");
    assert_eq!(data.write.as_deref(), Some("dir.write.data"));
}

#[test]
fn test_parse_efun_only_toml() {
    let toml = r#"
[efuns]
delete_file = "efun.file.delete"
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.efuns.get("delete_file").map(String::as_str), Some("efun.file.delete"));
    assert!(config.directories.is_empty());
}

#[test]
fn test_parse_directory_only_toml() {
    let toml = r#"
[directories]
"/cmds" = { write = "dir.write.cmds" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    assert!(config.efuns.is_empty());
    let cmds = config.directories.get("/cmds").unwrap();
    assert!(cmds.read.is_none());
    assert_eq!(cmds.write.as_deref(), Some("dir.write.cmds"));
}

// ─── dir_permission() — no match ─────────────────────────────────────────────

#[test]
fn test_dir_permission_unconfigured_path_allows() {
    let toml = r#"
[directories]
"/admin" = { write = "dir.write.admin" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    // /data is not in config at all
    assert!(config.dir_permission("/data/foo.txt", "write").is_none());
    assert!(config.dir_permission("/data/foo.txt", "read").is_none());
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
"/admin" = { read = "dir.read.admin", write = "dir.write.admin" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    assert_eq!(
        config.dir_permission("/admin/config.lua", "write").map(String::as_str),
        Some("dir.write.admin")
    );
    assert_eq!(
        config.dir_permission("/admin/subdir/file.lua", "read").map(String::as_str),
        Some("dir.read.admin")
    );
}

#[test]
fn test_dir_permission_longest_prefix_wins() {
    let toml = r#"
[directories]
"/data"       = { write = "dir.write.data" }
"/data/admin" = { write = "dir.write.data.admin" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    // Should match /data/admin (longer prefix)
    assert_eq!(
        config.dir_permission("/data/admin/secret.lua", "write").map(String::as_str),
        Some("dir.write.data.admin")
    );
    // Should match /data (shorter prefix)
    assert_eq!(
        config.dir_permission("/data/other.lua", "write").map(String::as_str),
        Some("dir.write.data")
    );
}

#[test]
fn test_dir_permission_read_open_when_only_write_configured() {
    let toml = r#"
[directories]
"/data" = { write = "dir.write.data" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    // Read is open (None = no restriction)
    assert!(config.dir_permission("/data/foo.txt", "read").is_none());
    // Write is restricted
    assert!(config.dir_permission("/data/foo.txt", "write").is_some());
}

#[test]
fn test_dir_permission_write_open_when_only_read_configured() {
    let toml = r#"
[directories]
"/logs" = { read = "dir.read.logs" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();

    assert!(config.dir_permission("/logs/server.log", "read").is_some());
    assert!(config.dir_permission("/logs/server.log", "write").is_none());
}

#[test]
fn test_dir_permission_unknown_op_returns_none() {
    let toml = r#"
[directories]
"/admin" = { read = "dir.read.admin", write = "dir.write.admin" }
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    // "execute" op is not defined — always returns None
    assert!(config.dir_permission("/admin/script.lua", "execute").is_none());
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
delete_file = "efun.file.delete"
"#;
    let config: PermissionConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.efuns.get("reload").map(String::as_str), Some("efun.reload"));
    assert_eq!(config.efuns.get("delete_file").map(String::as_str), Some("efun.file.delete"));
    assert!(config.efuns.get("send").is_none());
}
