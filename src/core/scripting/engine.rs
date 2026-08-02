use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use tokio::sync::mpsc;
use mlua::prelude::*;

use crate::error::Result;
use super::efuns::{EfunContext, set_current_session, get_current_session};
use super::debugger::{self, paths};

/// Commands sent from the async driver to the Lua thread
pub enum LuaCommand {
    /// A new session connected
    OnConnect { session_id: String },
    /// Input received from a session
    OnInput { session_id: String, text: String },
    /// A session disconnected
    OnDisconnect { session_id: String },
    /// GMCP message received
    OnGmcp { session_id: String, package: String, data: serde_json::Value },
    /// Admin: reload a Lua module
    Reload { module_name: String },
    /// A scheduled timer has fired
    TimerFired { id: String },
    /// Wake the Lua thread so it re-evaluates the debug hook.
    ///
    /// The thread otherwise parks in `blocking_recv` and would not notice a
    /// debug client attaching or detaching until some other event arrived.
    DebugSync,
    /// Shut down the Lua VM
    Shutdown,
}

/// The Lua scripting engine — runs on a dedicated thread.
pub struct ScriptEngine {
    /// Send commands to the Lua thread
    pub cmd_tx: mpsc::UnboundedSender<LuaCommand>,
    /// Handle to the Lua thread
    thread_handle: Option<JoinHandle<()>>,
}

impl ScriptEngine {
    /// Start the Lua VM on a dedicated thread. Non-blocking — returns immediately.
    pub fn start(
        mudlib_path: PathBuf,
        ctx: EfunContext,
        cmd_tx: mpsc::UnboundedSender<LuaCommand>,
        mut cmd_rx: mpsc::UnboundedReceiver<LuaCommand>,
    ) -> Result<Self> {
        let game_path = PathBuf::from(
            ctx.server_config.game.game_path.as_deref().unwrap_or("./game")
        );

        // Clone logger, session_handler and debug state before moving ctx into the thread
        let game_logger = ctx.game_logger.clone();
        let session_handler_for_log = ctx.session_handler.clone();
        let debug_state = ctx.debug_state.clone();

        let thread_handle = std::thread::spawn(move || {
            // Create LuaJIT VM. The `debug` stdlib is only loaded when the debug
            // adapter is enabled, because reaching it requires mlua's unsafe
            // constructor — `Lua::new_with` rejects StdLib::DEBUG outright.
            // It is hidden from `_G` before any mudlib code runs, so the game
            // still cannot see it.
            let lua = if debug_state.debug_library {
                tracing::warn!(
                    "Lua `debug` stdlib enabled for the debug adapter — mlua's safety \
                     guarantees are off. Do not run this configuration in production."
                );
                let lua = unsafe {
                    Lua::unsafe_new_with(
                        mlua::StdLib::ALL_SAFE | mlua::StdLib::DEBUG,
                        LuaOptions::default(),
                    )
                };
                if let Err(e) = debugger::introspect::hide_debug_library(&lua) {
                    tracing::error!("Failed to hide the debug library: {}", e);
                }
                if let Err(e) = debugger::introspect::load_helper(&lua) {
                    tracing::error!("Failed to load the introspection helper: {}", e);
                }
                lua
            } else {
                Lua::new()
            };

            // Register all efuns
            if let Err(e) = super::efuns::register_all(&lua, &ctx) {
                tracing::error!("Failed to register efuns: {}", e);
                return;
            }

            // Set package.path so require() finds modules relative to the mudlib
            // and game directories. Game path comes first so game files shadow mudlib.
            // These are the exact strings LuaJIT will use as `@`-prefixed chunk
            // names for every required file, so the debugger's path mapping keys
            // off the same helper.
            let mudlib_pkg_path = paths::abs_lua_path(&mudlib_path);
            let game_pkg_path = paths::abs_lua_path(&game_path);

            let new_path = format!(
                "{game}/?.lua;{game}/?/init.lua;{mudlib}/?.lua;{mudlib}/?/init.lua",
                game = game_pkg_path,
                mudlib = mudlib_pkg_path
            );
            tracing::debug!("Lua package.path prefix: {}", new_path);
            if let Err(e) = lua.load(format!(
                "package.path = \"{new_path};\" .. package.path",
                new_path = new_path.replace('"', "\\\"")
            ).as_str()).exec() {
                tracing::error!("Failed to set package.path: {}", e);
                return;
            }

            // Load the mudlib entry point
            let init_path = mudlib_path.join("init.lua");
            match std::fs::read_to_string(&init_path) {
                Ok(code) => {
                    if let Err(e) = lua.load(code.as_str())
                        .set_name(paths::chunk_name(&init_path))
                        .exec()
                    {
                        tracing::error!("Failed to load mudlib init.lua: {}", e);
                        log_lua_error(&game_logger, &session_handler_for_log, "init.lua", &e, None);
                        // Don't return — still accept connections
                    } else {
                        tracing::info!("Mudlib loaded from {}", init_path.display());
                    }
                }
                Err(e) => {
                    tracing::warn!("No mudlib init.lua found at {}: {}", init_path.display(), e);
                    tracing::warn!("Running with empty mudlib — define on_connect, on_input, on_disconnect globally.");
                }
            }

            // Load the game layer entry point (if present)
            let game_init_path = game_path.join("init.lua");
            match std::fs::read_to_string(&game_init_path) {
                Ok(code) => {
                    if let Err(e) = lua.load(code.as_str())
                        .set_name(paths::chunk_name(&game_init_path))
                        .exec()
                    {
                        tracing::error!("Failed to load game/init.lua: {}", e);
                        log_lua_error(&game_logger, &session_handler_for_log, "game/init.lua", &e, None);
                    } else {
                        tracing::info!("Game layer loaded from {}", game_init_path.display());
                    }
                }
                Err(_) => {
                    tracing::info!("No game/init.lua found — running without game layer");
                }
            }

            // Hook state for tracing / debugging. Lives outside the hook closure
            // (which is `Fn`, not `FnMut`) so counters and the intern table
            // survive a disarm/re-arm cycle.
            let hook_local = std::rc::Rc::new(std::cell::RefCell::new(debugger::HookLocal::new()));
            let mut installed_hook = debugger::InstalledHook::default();
            // The Lua thread is the only legitimate owner of the request channel:
            // requests are serviced from inside the hook while it is blocked.
            if let Some(rx) = debug_state.take_vm_rx() {
                hook_local.borrow_mut().attach_channel(rx);
            }

            // Event loop — receive commands from the async driver
            loop {
                match cmd_rx.blocking_recv() {
                    None => break, // Channel closed
                    Some(cmd) => {
                        // Arm or disarm the hook before running any Lua. This is the
                        // only place `set_hook` may be called — never from inside
                        // the hook itself.
                        debugger::sync_hook(&lua, &debug_state, &mut installed_hook, &hook_local);
                        match cmd {
                            LuaCommand::OnConnect { session_id } => {
                                set_current_session(Some(session_id.clone()));
                                debugger::set_dispatch_context(&debug_state, Some(&session_id));
                                dispatch_event(&lua, "on_connect", &[session_id], &game_logger, &session_handler_for_log);
                                debugger::set_dispatch_context(&debug_state, None);
                                set_current_session(None);
                            }
                            LuaCommand::OnInput { session_id, text } => {
                                set_current_session(Some(session_id.clone()));
                                debugger::set_dispatch_context(&debug_state, Some(&session_id));
                                debugger::hook::begin_dispatch(&hook_local);

                                let extra = Some(serde_json::json!({"input": &text[..text.len().min(100)]}));
                                dispatch_event_2(&lua, "on_input", &session_id, &text, &game_logger, &session_handler_for_log, extra);

                                // The verb is the first token; that is all the
                                // timing table needs, and reading it here keeps
                                // the mudlib dispatch path untouched.
                                let verb = text.split_whitespace().next().unwrap_or("");
                                debugger::hook::end_dispatch(&hook_local, &session_id, verb);
                                debugger::set_dispatch_context(&debug_state, None);
                                set_current_session(None);
                            }
                            LuaCommand::OnDisconnect { session_id } => {
                                set_current_session(Some(session_id.clone()));
                                dispatch_event(&lua, "on_disconnect", &[session_id], &game_logger, &session_handler_for_log);
                                set_current_session(None);
                            }
                            LuaCommand::OnGmcp { session_id, package, data } => {
                                set_current_session(Some(session_id.clone()));
                                let json_str = data.to_string();
                                dispatch_event_gmcp(&lua, &session_id, &package, &json_str, &game_logger, &session_handler_for_log);
                                set_current_session(None);
                            }
                            LuaCommand::Reload { module_name } => {
                                hot_reload(&lua, &module_name, &mudlib_path, &game_path);
                            }
                            LuaCommand::TimerFired { id } => {
                                dispatch_event(&lua, "on_timer", &[id], &game_logger, &session_handler_for_log);
                            }
                            // The sync_hook call above is the entire point.
                            LuaCommand::DebugSync => {}
                            LuaCommand::Shutdown => {
                                tracing::info!("Lua engine shutting down");
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(ScriptEngine {
            cmd_tx,
            thread_handle: Some(thread_handle),
        })
    }

    /// Send a command to the Lua thread. Non-blocking.
    pub fn send(&self, cmd: LuaCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Shut down the Lua VM and wait for the thread to finish.
    pub fn shutdown(mut self) {
        let _ = self.cmd_tx.send(LuaCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Log a Lua error to the structured journal.
fn log_lua_error(
    gl: &Arc<crate::core::logging::GameLogger>,
    sh: &Arc<std::sync::RwLock<crate::core::session::SessionHandler>>,
    fn_name: &str,
    error: &mlua::Error,
    extra_meta: Option<serde_json::Value>,
) {
    use crate::core::logging::JournalEntry;

    let err_str = error.to_string();
    let source = extract_lua_source(&err_str);

    let sid = get_current_session().unwrap_or_else(|| "unknown".to_string());
    let char_id = {
        if let Ok(id) = sid.parse::<crate::core::session::SessionId>() {
            sh.read().unwrap().get(&id).and_then(|s| s.state.character_id())
        } else {
            None
        }
    };

    let mut meta = serde_json::json!({
        "event":   fn_name,
        "sid":     sid,
        "char_id": char_id,
    });
    if let Some(extra) = extra_meta {
        if let (serde_json::Value::Object(ref mut base), serde_json::Value::Object(ext)) =
            (&mut meta, extra)
        {
            base.extend(ext);
        }
    }

    gl.journal(JournalEntry {
        level:   "error",
        source:  &source,
        message: &err_str,
        meta:    Some(meta),
    });
}

/// Try to extract a `filename:line` source label from a Lua error string.
fn extract_lua_source(err: &str) -> String {
    // `[string "X.lua"]:NN:` — chunks with no backing file (`load`ed strings).
    if let Some(rest) = err.strip_prefix("[string \"") {
        if let Some(end) = rest.find('"') {
            let filename = &rest[..end];
            let after = &rest[end..];
            if let Some(colon_pos) = after.find("]:") {
                let line_part = &after[colon_pos + 2..];
                if let Some(second_colon) = line_part.find(':') {
                    let line = &line_part[..second_colon];
                    return format!("{}:{}", filename, line.trim());
                }
            }
            return filename.to_string();
        }
    }
    // `<path>.lua:NN:` — file-backed chunks, which is every chunk now that
    // package.path and the three `set_name` sites all produce `@<abs path>`.
    if let Some(found) = extract_path_line(err) {
        return found;
    }
    // Fallback: first 60 chars
    err.chars().take(60).collect()
}

/// Pull `dir/file.lua:NN` out of an error string prefixed with an absolute path.
///
/// The path is shortened to its last two components — an absolute Windows path
/// would otherwise swamp the journal's `source` field.
fn extract_path_line(err: &str) -> Option<String> {
    let idx = err.find(".lua:")?;
    let line: String = err[idx + 5..].chars().take_while(char::is_ascii_digit).collect();
    if line.is_empty() {
        return None;
    }
    let path = &err[..idx + 4]; // through ".lua"
    let mut tail: Vec<&str> = path.rsplit(['/', '\\']).take(2).collect();
    tail.reverse();
    Some(format!("{}:{}", tail.join("/"), line))
}

#[cfg(test)]
mod tests {
    use super::extract_lua_source;

    #[test]
    fn extracts_source_from_a_file_backed_chunk() {
        // What LuaJIT produces for a chunk named `@C:/Code/oxigeon/mudlib/cmds/who.lua`.
        assert_eq!(
            extract_lua_source("C:/Code/oxigeon/mudlib/cmds/who.lua:42: attempt to index a nil value"),
            "cmds/who.lua:42"
        );
    }

    #[test]
    fn still_extracts_source_from_a_string_chunk() {
        assert_eq!(
            extract_lua_source("[string \"login.lua\"]:7: boom"),
            "login.lua:7"
        );
    }

    #[test]
    fn falls_back_when_there_is_no_source() {
        let err = "some error with no location at all";
        assert_eq!(extract_lua_source(err), err);
    }
}

/// Dispatch a single-arg (or multi-arg) Lua event with structured error logging.
fn dispatch_event(
    lua: &Lua,
    fn_name: &str,
    args: &[String],
    gl: &Arc<crate::core::logging::GameLogger>,
    sh: &Arc<std::sync::RwLock<crate::core::session::SessionHandler>>,
) {
    let func: LuaResult<LuaFunction> = lua.globals().get(fn_name);
    match func {
        Ok(f) => {
            let result = match args.len() {
                0 => f.call::<()>(()),
                1 => f.call::<()>(args[0].clone()),
                _ => f.call::<()>(args.to_vec()),
            };
            if let Err(e) = result {
                tracing::error!("Lua error in {}: {}", fn_name, e);
                log_lua_error(gl, sh, fn_name, &e, None);
            }
        }
        Err(_) => {
            tracing::trace!("No Lua function '{}' defined", fn_name);
        }
    }
}

/// Dispatch a two-arg Lua event with structured error logging.
fn dispatch_event_2(
    lua: &Lua,
    fn_name: &str,
    arg1: &str,
    arg2: &str,
    gl: &Arc<crate::core::logging::GameLogger>,
    sh: &Arc<std::sync::RwLock<crate::core::session::SessionHandler>>,
    extra_meta: Option<serde_json::Value>,
) {
    let func: LuaResult<LuaFunction> = lua.globals().get(fn_name);
    match func {
        Ok(f) => {
            if let Err(e) = f.call::<()>((arg1.to_string(), arg2.to_string())) {
                tracing::error!("Lua error in {}: {}", fn_name, e);
                log_lua_error(gl, sh, fn_name, &e, extra_meta);
            }
        }
        Err(_) => {
            tracing::trace!("No Lua function '{}' defined", fn_name);
        }
    }
}

/// Dispatch a GMCP event with structured error logging.
fn dispatch_event_gmcp(
    lua: &Lua,
    session_id: &str,
    package: &str,
    json: &str,
    gl: &Arc<crate::core::logging::GameLogger>,
    sh: &Arc<std::sync::RwLock<crate::core::session::SessionHandler>>,
) {
    let func: LuaResult<LuaFunction> = lua.globals().get("on_gmcp");
    match func {
        Ok(f) => {
            if let Err(e) = f.call::<()>((session_id.to_string(), package.to_string(), json.to_string())) {
                tracing::error!("Lua error in on_gmcp: {}", e);
                log_lua_error(gl, sh, "on_gmcp", &e,
                    Some(serde_json::json!({"package": package})));
            }
        }
        Err(_) => {}
    }
}

/// Hot-reload a Lua module by name.
///
/// Searches the game layer before the mudlib, mirroring `package.path` precedence
/// (game files shadow mudlib files), and names the reloaded chunk with its real
/// absolute path so error sources and debugger breakpoints keep resolving.
fn hot_reload(lua: &Lua, module_name: &str, mudlib_path: &Path, game_path: &Path) {
    tracing::info!("Hot-reloading Lua module: {}", module_name);

    let result: LuaResult<()> = (|| {
        // 1. Call on_unload hook if it exists
        if let Ok(hook) = lua.globals().get::<LuaFunction>("on_unload") {
            if let Err(e) = hook.call::<()>(module_name.to_string()) {
                tracing::warn!("on_unload hook error for {}: {}", module_name, e);
            }
        }

        // 2. Clear from package.loaded (both slash and dot variants)
        //    Lua's require() caches with dot-separated keys ("cmds.tasks")
        //    but users pass slash-separated paths ("cmds/tasks") to reload.
        let package: LuaTable = lua.globals().get("package")?;
        let loaded: LuaTable = package.get("loaded")?;
        let dot_key = module_name.replace('/', ".");
        loaded.set(module_name, LuaValue::Nil)?;
        if dot_key != module_name {
            loaded.set(dot_key.as_str(), LuaValue::Nil)?;
        }

        // 3. Find and load the file directly. Game layer wins, as it does in
        //    package.path — otherwise reloading a game file silently reloads the
        //    mudlib file it was shadowing, or fails outright.
        let safe_name = module_name.replace('.', "/");
        let relative = format!("{}.lua", safe_name);
        let file_path = [game_path, mudlib_path]
            .iter()
            .map(|root| root.join(&relative))
            .find(|p| p.is_file())
            .unwrap_or_else(|| mudlib_path.join(&relative));

        match std::fs::read_to_string(&file_path) {
            Ok(code) => {
                let module_val: LuaResult<LuaValue> = lua.load(code.as_str())
                    .set_name(paths::chunk_name(&file_path))
                    .call(());

                match module_val {
                    Ok(val) => {
                        loaded.set(module_name, val.clone())?;
                        if dot_key != module_name {
                            loaded.set(dot_key.as_str(), val)?;
                        }

                        // 4. Call on_load hook if it exists
                        if let Ok(hook) = lua.globals().get::<LuaFunction>("on_load") {
                            if let Err(e) = hook.call::<()>(module_name.to_string()) {
                                tracing::warn!("on_load hook error for {}: {}", module_name, e);
                            }
                        }

                        tracing::info!("Successfully reloaded module: {}", module_name);
                    }
                    Err(e) => {
                        tracing::error!("Failed to reload {} (old version kept): {}", module_name, e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Cannot read file {} for reload: {}", file_path.display(), e);
            }
        }

        Ok(())
    })();

    if let Err(e) = result {
        tracing::error!("Hot-reload failed for {}: {}", module_name, e);
    }
}

impl Drop for ScriptEngine {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(LuaCommand::Shutdown);
    }
}
