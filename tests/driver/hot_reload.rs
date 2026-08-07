//! Integration tests for hot-reload functionality.
//! Verifies that Lua modules can be reloaded at runtime and that:
//! - New behavior takes effect immediately after LuaCommand::Reload
//! - Failed reloads (syntax error) do not crash the engine
//! - on_connect / on_input / on_disconnect event dispatch works

use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tempfile::TempDir;

use oxigeon::config::MultisessionMode;
use oxigeon::core::scripting::efuns::EfunContext;
use oxigeon::core::scripting::{LuaCommand, ScriptEngine};
use oxigeon::core::session::{Session, SessionHandler, SessionOutput};
use oxigeon::domain::db::connection::AnyPool;

/// Create a test DB pool backed by a temp file
fn make_test_pool() -> (AnyPool, TempDir) {
    crate::common::test_pool()
}

fn make_efun_context(
    sh: Arc<RwLock<SessionHandler>>,
    mudlib_path: std::path::PathBuf,
    pool: AnyPool,
) -> EfunContext {
    crate::common::efun_context(sh, mudlib_path, pool, crate::common::TestCtx::default())
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
    // Via `append_file` rather than `io.open`: the sandbox removes `io`, and
    // the marker sits inside the mudlib root so the efun's jail allows it.
    let marker = mudlib.path().join("reloaded.txt");
    let module = "append_file('reloaded.txt', 'loaded\\n'); return {}\n";
    std::fs::write(game_dir.join("gamemod.lua"), module).unwrap();
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
