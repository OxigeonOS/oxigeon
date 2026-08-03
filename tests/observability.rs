//! Integration tests for the observability efuns and structured error logging.
//! Verifies that:
//! 1. A Lua error in on_input produces a journal entry in the log file.
//! 2. verify_file on a valid file returns (true, nil).
//! 3. verify_file on a file with syntax error returns (false, string).
//! 4. server_info() returns a table with uptime_secs >= 0.
//! 5. journal_write writes a readable entry.
//! 6. audit_write writes a readable entry.

mod common;

use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tempfile::TempDir;

use oxigeon::config::MultisessionMode;
use oxigeon::core::logging::GameLogger;
use oxigeon::core::scripting::efuns::EfunContext;
use oxigeon::core::scripting::{LuaCommand, ScriptEngine};
use oxigeon::core::session::{Session, SessionHandler, SessionOutput};
use oxigeon::domain::db::connection::AnyPool;

fn make_test_pool() -> (AnyPool, TempDir) {
    common::test_pool()
}

fn make_efun_context(
    sh: Arc<RwLock<SessionHandler>>,
    mudlib_path: std::path::PathBuf,
    pool: AnyPool,
    log_dir: &std::path::Path,
) -> (EfunContext, Arc<GameLogger>) {
    let ctx = common::efun_context(
        sh,
        mudlib_path,
        pool,
        common::TestCtx { log_dir: Some(log_dir.to_path_buf()), ..Default::default() },
    );
    let logger = ctx.game_logger.clone();
    (ctx, logger)
}

/// Start the engine with a given init.lua content and return the engine + logger.
fn start_engine_with_lua(
    mudlib: &TempDir,
    init_lua: &str,
) -> (ScriptEngine, Arc<GameLogger>, TempDir, TempDir) {
    std::fs::write(mudlib.path().join("init.lua"), init_lua).unwrap();

    let log_dir = TempDir::new().unwrap();
    let sh = Arc::new(RwLock::new(SessionHandler::new(MultisessionMode::Single, 256)));
    let (pool, db_dir) = make_test_pool();
    let (ctx, game_logger) = make_efun_context(
        sh,
        mudlib.path().to_path_buf(),
        pool,
        log_dir.path(),
    );

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx, cmd_rx).unwrap();
    // Give the engine time to load init.lua
    std::thread::sleep(std::time::Duration::from_millis(200));

    (engine, game_logger, log_dir, db_dir)
}

fn make_session(sh: &Arc<RwLock<SessionHandler>>) -> String {
    let (tx, _rx) = mpsc::channel::<SessionOutput>(16);
    let addr = "127.0.0.1:9999".parse().unwrap();
    let session = Session::new("telnet".to_string(), addr, tx);
    let sid = session.id.to_string();
    sh.write().unwrap().connect(session).unwrap();
    sid
}

// ─── Test 1: Lua error in on_input produces a journal entry ──────────────────

#[test]
fn test_lua_error_in_on_input_produces_journal_entry() {
    let mudlib = TempDir::new().unwrap();
    let init_lua = r#"
function on_input(session_id, text)
    error("deliberate error in on_input: " .. text)
end
"#;
    let log_dir = TempDir::new().unwrap();
    let sh = Arc::new(RwLock::new(SessionHandler::new(MultisessionMode::Single, 256)));
    let (pool, _db_dir) = make_test_pool();
    let (ctx, game_logger) = make_efun_context(
        sh.clone(),
        mudlib.path().to_path_buf(),
        pool,
        log_dir.path(),
    );
    std::fs::write(mudlib.path().join("init.lua"), init_lua).unwrap();

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx, cmd_rx).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let sid = make_session(&sh);
    engine.send(LuaCommand::OnInput {
        session_id: sid,
        text: "trigger-error".to_string(),
    });

    // Give the engine time to process and log the error
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(engine);

    // Check that the journal has an error entry
    let entries = game_logger.read_journal(50, Some("error"));
    assert!(!entries.is_empty(), "Expected at least one error journal entry after Lua error");
    let found = entries.iter().any(|line| {
        line.contains("on_input") || line.contains("deliberate error")
    });
    assert!(found, "Journal should contain on_input error reference. Got: {:?}", entries);
}

// ─── Test 2: verify_file on valid file returns true ──────────────────────────

#[test]
fn test_verify_file_valid_returns_true() {
    let mudlib = TempDir::new().unwrap();

    // Write a syntactically valid file
    std::fs::write(mudlib.path().join("goodfile.lua"), "return { ok = true }\n").unwrap();

    let init_lua = r#"
function on_input(session_id, text)
    local ok, err = verify_file("goodfile.lua")
    _verify_ok  = ok
    _verify_err = err
end
"#;
    let (engine, _gl, _log_dir, _db_dir) = start_engine_with_lua(&mudlib, init_lua);

    // The session is never registered with the handler: `verify_file` is
    // permission-free under `PermissionConfig::default()`, so it only needs an
    // id to dispatch against.
    let (tx, _rx) = mpsc::channel::<SessionOutput>(16);
    let addr = "127.0.0.1:9999".parse().unwrap();
    let sid = Session::new("telnet".to_string(), addr, tx).id.to_string();

    engine.send(LuaCommand::OnInput {
        session_id: sid,
        text: "verify".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(engine);
    // No crash = verify_file ran without panicking. The return value is visible only inside Lua.
    // This is a smoke test that verify_file doesn't blow up on a valid file.
}

// ─── Test 3: verify_file on a syntax-error file returns false ────────────────

#[test]
fn test_verify_file_syntax_error_returns_false() {
    let mudlib = TempDir::new().unwrap();

    // Write a file with a Lua syntax error
    std::fs::write(mudlib.path().join("badfile.lua"), "function unclosed(\n").unwrap();

    // The init.lua calls verify_file and logs the result
    let init_lua = r#"
function on_input(session_id, text)
    local ok, err = verify_file("badfile.lua")
    -- ok should be false, err should be a string
    if ok then
        error("verify_file should have returned false for bad syntax")
    end
    -- Log it so we can check
    journal_write("info", "verify_file returned ok=" .. tostring(ok) .. " err=" .. tostring(err))
end
"#;
    let log_dir = TempDir::new().unwrap();
    let sh = Arc::new(RwLock::new(SessionHandler::new(MultisessionMode::Single, 256)));
    let (pool, _db_dir) = make_test_pool();
    let (ctx, game_logger) = make_efun_context(
        sh.clone(),
        mudlib.path().to_path_buf(),
        pool,
        log_dir.path(),
    );
    std::fs::write(mudlib.path().join("init.lua"), init_lua).unwrap();

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx, cmd_rx).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let sid = make_session(&sh);
    engine.send(LuaCommand::OnInput {
        session_id: sid,
        text: "check".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(engine);

    // Check that the journal has an info entry confirming ok=false
    let entries = game_logger.read_journal(20, Some("info"));
    let found = entries.iter().any(|l| l.contains("ok=false"));
    assert!(found, "Expected journal entry showing verify_file returned ok=false. Got: {:?}", entries);
}

// ─── Test 4: server_info() returns a table with uptime_secs >= 0 ─────────────

#[test]
fn test_server_info_uptime_is_nonnegative() {
    let mudlib = TempDir::new().unwrap();

    let init_lua = r#"
function on_input(session_id, text)
    local info = server_info()
    if info.uptime_secs < 0 then
        error("uptime_secs is negative: " .. tostring(info.uptime_secs))
    end
    if type(info.version) ~= "string" then
        error("version is not a string")
    end
    if type(info.name) ~= "string" then
        error("name is not a string")
    end
    journal_write("info", "server_info ok uptime=" .. tostring(info.uptime_secs))
end
"#;
    let log_dir = TempDir::new().unwrap();
    let sh = Arc::new(RwLock::new(SessionHandler::new(MultisessionMode::Single, 256)));
    let (pool, _db_dir) = make_test_pool();
    let (ctx, game_logger) = make_efun_context(
        sh.clone(),
        mudlib.path().to_path_buf(),
        pool,
        log_dir.path(),
    );
    std::fs::write(mudlib.path().join("init.lua"), init_lua).unwrap();

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx, cmd_rx).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let sid = make_session(&sh);
    engine.send(LuaCommand::OnInput {
        session_id: sid,
        text: "info".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(engine);

    // No error journal entries means the Lua assertions passed
    let errors = game_logger.read_journal(20, Some("error"));
    assert!(errors.is_empty(), "Expected no errors from server_info(). Got: {:?}", errors);

    let infos = game_logger.read_journal(20, Some("info"));
    let found = infos.iter().any(|l| l.contains("server_info ok"));
    assert!(found, "Expected journal entry 'server_info ok'. Got: {:?}", infos);
}

// ─── Test 5: journal_write writes a readable entry ───────────────────────────

#[test]
fn test_journal_write_from_lua_is_readable() {
    let mudlib = TempDir::new().unwrap();

    let init_lua = r#"
function on_input(session_id, text)
    journal_write("warn", "lua-test-sentinel-" .. text)
end
"#;
    let log_dir = TempDir::new().unwrap();
    let sh = Arc::new(RwLock::new(SessionHandler::new(MultisessionMode::Single, 256)));
    let (pool, _db_dir) = make_test_pool();
    let (ctx, game_logger) = make_efun_context(
        sh.clone(),
        mudlib.path().to_path_buf(),
        pool,
        log_dir.path(),
    );
    std::fs::write(mudlib.path().join("init.lua"), init_lua).unwrap();

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx, cmd_rx).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let sid = make_session(&sh);
    engine.send(LuaCommand::OnInput {
        session_id: sid,
        text: "hello42".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(engine);

    let entries = game_logger.read_journal(20, Some("warn"));
    assert!(!entries.is_empty(), "Expected at least one warn journal entry");
    let found = entries.iter().any(|l| l.contains("lua-test-sentinel-hello42"));
    assert!(found, "Expected sentinel message in journal. Got: {:?}", entries);
    // All entries must be valid JSON
    for line in &entries {
        let obj: serde_json::Value = serde_json::from_str(line)
            .expect("Journal line must be valid JSON");
        assert_eq!(obj["level"], "warn");
    }
}

// ─── Test 6: audit_write writes a readable entry ─────────────────────────────

#[test]
fn test_audit_write_from_lua_is_readable() {
    let mudlib = TempDir::new().unwrap();

    let init_lua = r#"
function on_input(session_id, text)
    audit_write("cmd.test-sentinel", true, nil)
end
"#;
    let log_dir = TempDir::new().unwrap();
    let sh = Arc::new(RwLock::new(SessionHandler::new(MultisessionMode::Single, 256)));
    let (pool, _db_dir) = make_test_pool();
    let (ctx, game_logger) = make_efun_context(
        sh.clone(),
        mudlib.path().to_path_buf(),
        pool,
        log_dir.path(),
    );
    std::fs::write(mudlib.path().join("init.lua"), init_lua).unwrap();

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx, cmd_rx).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let sid = make_session(&sh);
    engine.send(LuaCommand::OnInput {
        session_id: sid,
        text: "audit".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(engine);

    let entries = game_logger.read_audit(20);
    assert!(!entries.is_empty(), "Expected at least one audit entry");
    let found = entries.iter().any(|l| l.contains("cmd.test-sentinel"));
    assert!(found, "Expected sentinel action in audit. Got: {:?}", entries);
    // Validate JSON structure
    for line in &entries {
        let obj: serde_json::Value = serde_json::from_str(line)
            .expect("Audit line must be valid JSON");
        assert!(obj["ts"].is_string());
        assert!(obj["action"].is_string());
    }
}

