use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use tokio::sync::mpsc;
use mlua::prelude::*;

use crate::error::Result;
use super::efuns::{EfunContext, set_current_session, get_current_session};

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
        let mudlib_str = mudlib_path.to_string_lossy().to_string();

        // Clone logger and session_handler before moving ctx into the thread
        let game_logger = ctx.game_logger.clone();
        let session_handler_for_log = ctx.session_handler.clone();

        let thread_handle = std::thread::spawn(move || {
            // Create LuaJIT VM
            let lua = Lua::new();

            // Register all efuns
            if let Err(e) = super::efuns::register_all(&lua, &ctx) {
                tracing::error!("Failed to register efuns: {}", e);
                return;
            }

            // Set package.path so require() finds modules relative to the mudlib directory.
            let mudlib_canon = mudlib_path
                .canonicalize()
                .unwrap_or_else(|_| mudlib_path.clone());
            let mut mudlib_pkg_path = mudlib_canon.to_string_lossy().replace('\\', "/");
            if mudlib_pkg_path.starts_with("//?/") {
                mudlib_pkg_path = mudlib_pkg_path[4..].to_string();
            }
            let new_path = format!(
                "{mudlib}/?.lua;{mudlib}/?/init.lua",
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
                        .set_name("init.lua")
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

            // Event loop — receive commands from the async driver
            loop {
                match cmd_rx.blocking_recv() {
                    None => break, // Channel closed
                    Some(cmd) => {
                        match cmd {
                            LuaCommand::OnConnect { session_id } => {
                                set_current_session(Some(session_id.clone()));
                                dispatch_event(&lua, "on_connect", &[session_id], &game_logger, &session_handler_for_log);
                                set_current_session(None);
                            }
                            LuaCommand::OnInput { session_id, text } => {
                                set_current_session(Some(session_id.clone()));
                                let extra = Some(serde_json::json!({"input": &text[..text.len().min(100)]}));
                                dispatch_event_2(&lua, "on_input", &session_id, &text, &game_logger, &session_handler_for_log, extra);
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
                                hot_reload(&lua, &module_name, &mudlib_str);
                            }
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
    // Patterns: `[string "X.lua"]:NN:` or `X.lua:NN:`
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
    // Fallback: first 60 chars
    err.chars().take(60).collect()
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
fn hot_reload(lua: &Lua, module_name: &str, mudlib_path: &str) {
    tracing::info!("Hot-reloading Lua module: {}", module_name);

    let result: LuaResult<()> = (|| {
        // 1. Call on_unload hook if it exists
        if let Ok(hook) = lua.globals().get::<LuaFunction>("on_unload") {
            if let Err(e) = hook.call::<()>(module_name.to_string()) {
                tracing::warn!("on_unload hook error for {}: {}", module_name, e);
            }
        }

        // 2. Clear from package.loaded
        let package: LuaTable = lua.globals().get("package")?;
        let loaded: LuaTable = package.get("loaded")?;
        loaded.set(module_name, LuaValue::Nil)?;

        // 3. Find and load the file directly
        let safe_name = module_name.replace('.', "/");
        let file_path = format!("{}/{}.lua", mudlib_path, safe_name);

        match std::fs::read_to_string(&file_path) {
            Ok(code) => {
                let module_val: LuaResult<LuaValue> = lua.load(code.as_str())
                    .set_name(module_name)
                    .call(());

                match module_val {
                    Ok(val) => {
                        loaded.set(module_name, val)?;

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
                tracing::error!("Cannot read file {} for reload: {}", file_path, e);
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
