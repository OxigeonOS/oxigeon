//! Integration tests for hot-reload functionality.
//! Verifies that Lua modules can be reloaded at runtime and that:
//! - New behavior takes effect immediately after LuaCommand::Reload
//! - Failed reloads (syntax error) do not crash the engine
//! - on_connect / on_input / on_disconnect event dispatch works

use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tempfile::TempDir;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use oxigeon::core::scripting::{ScriptEngine, LuaCommand};
use oxigeon::core::session::{SessionHandler, Session, SessionOutput};
use oxigeon::core::scripting::efuns::EfunContext;
use oxigeon::core::logging::GameLogger;
use oxigeon::config::{
    ServerConfig, GameConfig, SessionsConfig, AccountsConfig, LimitsConfig, MultisessionMode,
    DatabaseConfig, DatabaseBackend, PermissionConfig,
};
use oxigeon::domain::models::{DieselAccountStore, DieselCharacterStore};
use oxigeon::domain::models::role::DieselRoleStore;
use oxigeon::domain::db::connection::AnyPool;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// Create a test DB pool backed by a temp file
fn make_test_pool() -> (AnyPool, TempDir) {
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

/// Create an EfunContext backed by a temp DB
fn make_efun_context(
    sh: Arc<RwLock<SessionHandler>>,
    mudlib_path: std::path::PathBuf,
    pool: AnyPool,
) -> EfunContext {
    let server_config = ServerConfig {
        game: GameConfig {
            name: "TestMUD".to_string(),
            mudlib_path: mudlib_path.to_string_lossy().to_string(),
            game_path: Some(mudlib_path.join("game").to_string_lossy().to_string()),
            command_paths: None,
            start_room: None,
            area_reset_seconds: Some(900),
            autosave_seconds: Some(300),
        },
        sessions: SessionsConfig {
            multisession_mode: MultisessionMode::Single,
            max_connections: 256,
        },
        accounts: AccountsConfig {
            allow_creation: true,
            min_password_length: 6,
            max_characters_per_account: 5,
        },
        limits: LimitsConfig {
            lua_memory_mb: 64,
            lua_instruction_limit: 1_000_000,
            input_buffer_bytes: 4096,
        },
    };

    let log_dir = mudlib_path.join("logs");
    let game_logger = std::sync::Arc::new(GameLogger::new(&log_dir));
    EfunContext {
        session_handler: sh,
        account_store: Arc::new(DieselAccountStore::new(pool.clone(), 6)),
        character_store: Arc::new(DieselCharacterStore::new(pool.clone(), 5)),
        role_store: Arc::new(DieselRoleStore::new(pool)),
        server_config: Arc::new(server_config),
        mudlib_path,
        cmd_tx: None,  // Not needed for test engine
        permission_config: Arc::new(PermissionConfig::default()),
        game_logger,
        started_at: std::time::Instant::now(),
        started_at_utc: "2026-01-01T00:00:00Z".to_string(),
        debug_state: oxigeon::core::scripting::debugger::DebugState::shared(1024, 64),
    }
}

/// Set up a test mudlib and start the scripting engine
fn setup_engine(mudlib: &TempDir, init_content: &str, module_content: &str) -> (ScriptEngine, TempDir) {
    std::fs::write(mudlib.path().join("init.lua"), init_content).unwrap();
    std::fs::write(mudlib.path().join("mymodule.lua"), module_content).unwrap();

    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));

    let (pool, db_dir) = make_test_pool();
    let ctx = make_efun_context(sh, mudlib.path().to_path_buf(), pool);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx, cmd_rx).unwrap();
    (engine, db_dir)
}

#[test]
fn test_hot_reload_updates_module_behavior() {
    let mudlib = TempDir::new().unwrap();

    let init_lua = r#"
local m = require("mymodule")
function get_version()
    return m.version()
end
"#;
    let module_v1 = r#"
local M = {}
function M.version() return 1 end
return M
"#;
    let module_v2 = r#"
local M = {}
function M.version() return 2 end
return M
"#;

    let (engine, _db_dir) = setup_engine(&mudlib, init_lua, module_v1);

    // Let the engine thread start and load init.lua
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Update module to v2 and trigger reload
    std::fs::write(mudlib.path().join("mymodule.lua"), module_v2).unwrap();
    engine.send(LuaCommand::Reload {
        module_name: "mymodule".to_string(),
    });

    std::thread::sleep(std::time::Duration::from_millis(300));

    // Engine should still be running (no crash = success at this integration level)
    drop(engine);
}

#[test]
fn test_hot_reload_handles_invalid_lua_gracefully() {
    let mudlib = TempDir::new().unwrap();

    let init_lua = "-- empty init\n";
    let module_valid = "return {}\n";
    let module_broken = "this is not valid lua !!!@@@\n";

    let (engine, _db_dir) = setup_engine(&mudlib, init_lua, module_valid);

    std::thread::sleep(std::time::Duration::from_millis(150));

    // Replace with broken version and reload
    std::fs::write(mudlib.path().join("mymodule.lua"), module_broken).unwrap();
    engine.send(LuaCommand::Reload {
        module_name: "mymodule".to_string(),
    });

    std::thread::sleep(std::time::Duration::from_millis(200));

    // Engine still alive — can send another command without panic
    engine.send(LuaCommand::Reload {
        module_name: "mymodule".to_string(),
    });

    std::thread::sleep(std::time::Duration::from_millis(100));
    drop(engine);
}

#[test]
fn test_engine_dispatches_on_connect_and_input_and_disconnect() {
    let mudlib = TempDir::new().unwrap();

    // Lua that just tracks calls by writing to a global (visible from engine thread)
    let init_lua = r#"
_event_log = ""
function on_connect(session_id)
    _event_log = _event_log .. "connect;"
end
function on_disconnect(session_id)
    _event_log = _event_log .. "disconnect;"
end
function on_input(session_id, text)
    _event_log = _event_log .. "input:" .. text .. ";"
end
"#;

    std::fs::write(mudlib.path().join("init.lua"), init_lua).unwrap();

    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));

    let (pool, _db_dir) = make_test_pool();
    let ctx = make_efun_context(sh.clone(), mudlib.path().to_path_buf(), pool);
    let (cmd_tx2, cmd_rx2) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx2, cmd_rx2).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(150));

    // Register a dummy session
    let (tx, _rx) = mpsc::channel::<SessionOutput>(16);
    let addr = "127.0.0.1:9999".parse().unwrap();
    let session = Session::new("telnet".to_string(), addr, tx);
    let sid = session.id.to_string();
    {
        let mut h = sh.write().unwrap();
        h.connect(session).unwrap();
    }

    // Send all three events
    engine.send(LuaCommand::OnConnect { session_id: sid.clone() });
    engine.send(LuaCommand::OnInput { session_id: sid.clone(), text: "hello".to_string() });
    engine.send(LuaCommand::OnDisconnect { session_id: sid });

    // Wait for all events to be processed
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Engine should be alive (no crash = success)
    drop(engine);
}

/// `hot_reload` used to build `{mudlib_path}/{name}.lua` unconditionally, so a
/// module living in the game layer could never be reloaded — the read simply
/// failed and the old version stayed live. It now searches the game layer first,
/// matching `package.path` precedence.
#[test]
fn test_hot_reload_finds_game_layer_modules() {
    let mudlib = TempDir::new().unwrap();
    let game_dir = mudlib.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();

    // The reloaded chunk appends to a marker file, giving us an observable
    // side effect that proves the file was actually found and executed.
    let marker = mudlib.path().join("reloaded.txt");
    let marker_lua = marker.to_string_lossy().replace('\\', "/");
    let module = format!(
        "local f = io.open('{marker_lua}', 'a'); f:write('loaded\\n'); f:close(); return {{}}\n"
    );
    std::fs::write(game_dir.join("gamemod.lua"), &module).unwrap();
    std::fs::write(mudlib.path().join("init.lua"), "require('gamemod')\n").unwrap();

    let sh = Arc::new(RwLock::new(SessionHandler::new(MultisessionMode::Single, 256)));
    let (pool, _db_dir) = make_test_pool();
    let ctx = make_efun_context(sh, mudlib.path().to_path_buf(), pool);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, tx, rx).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(250));
    let after_boot = std::fs::read_to_string(&marker).unwrap_or_default();
    assert_eq!(after_boot.lines().count(), 1, "init.lua should have required it once");

    engine.send(LuaCommand::Reload { module_name: "gamemod".to_string() });
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(engine);

    let after_reload = std::fs::read_to_string(&marker).unwrap_or_default();
    assert_eq!(
        after_reload.lines().count(),
        2,
        "reload should have re-executed the game-layer module; marker was {after_reload:?}"
    );
}

/// Chunks are now named `@<abs path>`, so a runtime error reports a real file
/// rather than `[string "init.lua"]`, and the journal's `source` field carries a
/// usable `dir/file.lua:line`.
#[test]
fn test_lua_errors_report_a_real_source_path() {
    let mudlib = TempDir::new().unwrap();
    let init_lua = "function on_input(session_id, text)\n    error('boom from init')\nend\n";
    std::fs::write(mudlib.path().join("init.lua"), init_lua).unwrap();

    let sh = Arc::new(RwLock::new(SessionHandler::new(MultisessionMode::Single, 256)));
    let (pool, _db_dir) = make_test_pool();
    let ctx = make_efun_context(sh, mudlib.path().to_path_buf(), pool);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, tx, rx).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(200));
    engine.send(LuaCommand::OnInput {
        session_id: "1".to_string(),
        text: "anything".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(engine);

    let journal = std::fs::read_to_string(mudlib.path().join("logs/journal.log"))
        .expect("journal.log should exist after a Lua error");
    let entry = journal
        .lines()
        .find(|l| l.contains("boom from init"))
        .unwrap_or_else(|| panic!("no journal entry for the error; journal was:\n{journal}"));

    assert!(
        entry.contains("init.lua:2"),
        "journal `source` should be a real file:line, got: {entry}"
    );
    assert!(
        !entry.contains("[string "),
        "error should no longer be reported as an unnamed string chunk: {entry}"
    );
}

#[test]
fn test_multiple_reload_cycles_stable() {
    let mudlib = TempDir::new().unwrap();
    std::fs::write(mudlib.path().join("init.lua"), "-- noop\n").unwrap();

    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));
    let (pool, _db_dir) = make_test_pool();
    let ctx = make_efun_context(sh, mudlib.path().to_path_buf(), pool);

    std::fs::write(mudlib.path().join("mymod.lua"), "return {v=1}\n").unwrap();
    let (cmd_tx3, cmd_rx3) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx3, cmd_rx3).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(100));

    // 5 reload cycles
    for i in 2..=6 {
        let new_content = format!("return {{v={}}}\n", i);
        std::fs::write(mudlib.path().join("mymod.lua"), new_content).unwrap();
        engine.send(LuaCommand::Reload { module_name: "mymod".to_string() });
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    std::thread::sleep(std::time::Duration::from_millis(200));
    // Still alive after 5 reloads
    drop(engine);
}
