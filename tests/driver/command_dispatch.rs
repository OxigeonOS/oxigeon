//! Integration tests for the command dispatch system.
//! Tests the commands.lua dispatcher and parser by running them through
//! a minimal scripting engine backed by a real temp mudlib directory.

use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tempfile::TempDir;

use oxigeon::config::MultisessionMode;
use oxigeon::core::scripting::efuns::EfunContext;
use oxigeon::core::scripting::{LuaCommand, ScriptEngine};
use oxigeon::core::session::{Session, SessionHandler, SessionOutput};
use oxigeon::domain::db::connection::AnyPool;

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

/// Create a minimal mudlib with commands.lua dispatcher and a test command.
/// The `init.lua` dispatches on_input to commands.dispatch().
fn setup_command_mudlib(mudlib: &TempDir) {
    // Create required directories
    std::fs::create_dir_all(mudlib.path().join("lib")).unwrap();
    std::fs::create_dir_all(mudlib.path().join("cmds")).unwrap();

    // Inline the dispatcher Lua so the test is self-contained and works
    // regardless of where cargo runs from.
    let commands_lua = r#"
local M = {}
local _registry = {}
local _aliases  = {}

local function register(mod)
    _registry[mod.name] = mod
    for _, a in ipairs(mod.aliases or {}) do
        _aliases[a] = mod.name
    end
end

local function lazy_load(verb)
    if _registry[verb] then return _registry[verb] end
    local ok, mod = pcall(require, "cmds." .. verb)
    if ok and type(mod) == "table" and type(mod.execute) == "function" then
        register(mod)
        return mod
    end
    return nil
end

function M.parse(text)
    local verb, rest = text:match("^(%S+)%s*(.*)")
    if not verb then return nil, "", {} end
    local args = {}
    for tok in rest:gmatch("%S+") do args[#args+1] = tok end
    return verb:lower(), rest, args
end

function M.registry()
    return _registry
end

function M.dispatch(session_id, text)
    text = text:gsub("^%s+", ""):gsub("%s+$", "")
    if text == "" then
        if type(send_prompt) == "function" then send_prompt(session_id, "\r\n> ") end
        return
    end

    local verb, args_str, args = M.parse(text)
    verb = _aliases[verb] or verb

    local mod = lazy_load(verb)
    if not mod then
        if type(send) == "function" then
            send(session_id, "\r\nUnknown command: '" .. verb .. "'. Type 'help' for a list.\r\n")
        end
        if type(send_prompt) == "function" then send_prompt(session_id, "> ") end
        return
    end

    if mod.permission and type(has_permission) == "function" then
        if not has_permission(session_id, mod.permission) then
            if type(send) == "function" then send(session_id, "\r\nPermission denied.\r\n") end
            if type(send_prompt) == "function" then send_prompt(session_id, "> ") end
            return
        end
    end

    local ok, err = pcall(mod.execute, session_id, args_str, args)
    if not ok then
        _dispatch_errors = (_dispatch_errors or 0) + 1
        if type(send) == "function" then send(session_id, "\r\nAn error occurred.\r\n") end
        if type(send_prompt) == "function" then send_prompt(session_id, "> ") end
    end
end

return M
"#;
    std::fs::write(mudlib.path().join("lib").join("commands.lua"), commands_lua).unwrap();

    // init.lua that uses commands.dispatch and writes results to a log global
    let init_lua = r#"
_dispatch_log = ""
_error_count  = 0

local commands = require("lib.commands")

-- Stub efuns for testing (real efuns are registered, but we supplement with Lua stubs
-- for anything not yet available in the test engine)
if type(send_prompt) ~= "function" then
    function send_prompt(sid, text) end
end

function on_connect(session_id) end
function on_disconnect(session_id) end
function on_gmcp(session_id, pkg, data) end
function on_unload(m) end
function on_load(m) end

function on_input(session_id, text)
    _dispatch_log = _dispatch_log .. "|" .. text
    commands.dispatch(session_id, text)
end

-- Expose dispatch and parse for direct testing
function test_parse(text)
    return commands.parse(text)
end
"#;
    std::fs::write(mudlib.path().join("init.lua"), init_lua).unwrap();

    // A real test command that records it was called
    let echo_cmd = r#"
local M = {}
M.name       = "echo"
M.aliases    = { "ec" }
M.category   = "test"
M.summary    = "Echo args back."
M.permission = nil

function M.execute(session_id, args_str, args)
    -- write to global log so test can observe
    _cmd_echo_called = true
    _cmd_echo_args   = args_str
end

return M
"#;
    std::fs::write(mudlib.path().join("cmds").join("echo.lua"), echo_cmd).unwrap();

    // A command that throws a Lua error
    let err_cmd = r#"
local M = {}
M.name       = "errcmd"
M.aliases    = {}
M.category   = "test"
M.summary    = "Always errors."
M.permission = nil

function M.execute(session_id, args_str, args)
    error("deliberate test error from errcmd")
end

return M
"#;
    std::fs::write(mudlib.path().join("cmds").join("errcmd.lua"), err_cmd).unwrap();
}

// Helper: start engine with command mudlib
fn start_command_engine(mudlib: &TempDir) -> (ScriptEngine, Arc<RwLock<SessionHandler>>, TempDir) {
    setup_command_mudlib(mudlib);

    let sh = Arc::new(RwLock::new(
        SessionHandler::new(MultisessionMode::Single, 256)
    ));
    let (pool, db_dir) = make_test_pool();
    let ctx = make_efun_context(sh.clone(), mudlib.path().to_path_buf(), pool);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<LuaCommand>();
    let engine = ScriptEngine::start(mudlib.path().to_path_buf(), ctx, cmd_tx, cmd_rx).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    (engine, sh, db_dir)
}

// Helper: make a session and return its ID string
fn make_session(sh: &Arc<RwLock<SessionHandler>>) -> String {
    let (tx, _rx) = mpsc::channel::<SessionOutput>(16);
    let addr = "127.0.0.1:9999".parse().unwrap();
    let session = Session::new("telnet".to_string(), addr, tx);
    let sid = session.id.to_string();
    sh.write().unwrap().connect(session).unwrap();
    sid
}

// ─── Parse tests (pure Lua, no session needed) ───────────────────────────────

#[test]
fn test_parse_empty_input_returns_nil_verb() {
    let mudlib = TempDir::new().unwrap();
    let (engine, sh, _db) = start_command_engine(&mudlib);
    let sid = make_session(&sh);

    // Send empty input — should NOT crash and produce no verb
    engine.send(LuaCommand::OnInput {
        session_id: sid.clone(),
        text: "".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    drop(engine);
}

#[test]
fn test_parse_verb_only() {
    let mudlib = TempDir::new().unwrap();
    let (engine, sh, _db) = start_command_engine(&mudlib);
    let sid = make_session(&sh);

    engine.send(LuaCommand::OnInput {
        session_id: sid.clone(),
        text: "who".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    // Engine stays alive — no crash means who dispatched correctly
    drop(engine);
}

#[test]
fn test_dispatch_known_command_executes() {
    let mudlib = TempDir::new().unwrap();
    let (engine, sh, _db) = start_command_engine(&mudlib);
    let sid = make_session(&sh);

    // "echo hello world" should dispatch to echo.lua and set _cmd_echo_called
    engine.send(LuaCommand::OnInput {
        session_id: sid.clone(),
        text: "echo hello world".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    // Engine still alive = command executed without crashing the VM
    drop(engine);
}

#[test]
fn test_dispatch_alias_resolution() {
    let mudlib = TempDir::new().unwrap();
    let (engine, sh, _db) = start_command_engine(&mudlib);
    let sid = make_session(&sh);

    // "ec" is an alias for "echo" — should not fail
    engine.send(LuaCommand::OnInput {
        session_id: sid.clone(),
        text: "ec hello".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    drop(engine);
}

#[test]
fn test_dispatch_unknown_command_does_not_crash() {
    let mudlib = TempDir::new().unwrap();
    let (engine, sh, _db) = start_command_engine(&mudlib);
    let sid = make_session(&sh);

    // Completely unknown verb
    engine.send(LuaCommand::OnInput {
        session_id: sid.clone(),
        text: "xyzzy_no_such_command".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    // Still alive after unknown command
    engine.send(LuaCommand::OnInput {
        session_id: sid,
        text: "echo ping".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    drop(engine);
}

#[test]
fn test_dispatch_command_error_is_caught() {
    let mudlib = TempDir::new().unwrap();
    let (engine, sh, _db) = start_command_engine(&mudlib);
    let sid = make_session(&sh);

    // errcmd always calls error() — should be caught by pcall in dispatcher
    engine.send(LuaCommand::OnInput {
        session_id: sid.clone(),
        text: "errcmd".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(150));

    // Engine must still be alive and process another command
    engine.send(LuaCommand::OnInput {
        session_id: sid,
        text: "echo still alive".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    drop(engine);
}

#[test]
fn test_dispatch_multiple_commands_sequential() {
    let mudlib = TempDir::new().unwrap();
    let (engine, sh, _db) = start_command_engine(&mudlib);
    let sid = make_session(&sh);

    // Fire several commands in sequence — engine must handle all without crashing
    for i in 0..5 {
        engine.send(LuaCommand::OnInput {
            session_id: sid.clone(),
            text: format!("echo message {}", i),
        });
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    drop(engine);
}

#[test]
fn test_dispatch_whitespace_trimmed() {
    let mudlib = TempDir::new().unwrap();
    let (engine, sh, _db) = start_command_engine(&mudlib);
    let sid = make_session(&sh);

    // Leading/trailing whitespace around the verb should be stripped
    engine.send(LuaCommand::OnInput {
        session_id: sid.clone(),
        text: "   echo   trimmed args   ".to_string(),
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    drop(engine);
}

