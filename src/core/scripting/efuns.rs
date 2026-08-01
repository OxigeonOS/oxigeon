use std::sync::{Arc, RwLock, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use mlua::prelude::*;
use serde_json::Value as JsonValue;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::server_config::ServerConfig;
use crate::config::permissions_config::PermissionConfig;
use crate::core::session::{SessionHandler, SessionOutput, SessionId};
use crate::core::scripting::engine::LuaCommand;
use crate::core::logging::{GameLogger, AuditEntry, JournalEntry};
use crate::domain::models::{DieselAccountStore, DieselCharacterStore};
use crate::domain::models::role::DieselRoleStore;

/// Context passed to Lua efun closures — shared handles to driver subsystems
#[derive(Clone)]
pub struct EfunContext {
    pub session_handler:    Arc<RwLock<SessionHandler>>,
    pub account_store:      Arc<DieselAccountStore>,
    pub character_store:    Arc<DieselCharacterStore>,
    pub role_store:         Arc<DieselRoleStore>,
    pub server_config:      Arc<ServerConfig>,
    pub mudlib_path:        PathBuf,
    /// Channel back to the engine for Lua-triggered reloads
    pub cmd_tx:             Option<UnboundedSender<LuaCommand>>,
    pub permission_config:  Arc<PermissionConfig>,
    pub game_logger:        Arc<GameLogger>,
    pub started_at:         Instant,
    pub started_at_utc:     String,  // ISO 8601 captured at startup
}

// The currently-active session ID for the Lua thread.
// Set by the engine before dispatching each event to Lua.
// This is a thread-local since the Lua VM runs on a single dedicated thread.
thread_local! {
    static CURRENT_SESSION: std::cell::RefCell<Option<String>> =
        std::cell::RefCell::new(None);
}

pub fn set_current_session(id: Option<String>) {
    CURRENT_SESSION.with(|s| *s.borrow_mut() = id);
}

pub fn get_current_session() -> Option<String> {
    CURRENT_SESSION.with(|s| s.borrow().clone())
}

/// Resolve (session_id_str, character_name) for audit entries.
/// Returns ("unknown", "") if the session isn't found.
fn resolve_session_char(
    sid: Option<&str>,
    sh: &Arc<RwLock<SessionHandler>>,
) -> (String, String) {
    let Some(sid_str) = sid else {
        return ("unknown".to_string(), "".to_string());
    };
    let id: SessionId = match sid_str.parse() {
        Ok(id) => id,
        Err(_) => return (sid_str.to_string(), "".to_string()),
    };
    let handler = sh.read().unwrap();
    let char_id = handler.get(&id).and_then(|s| s.state.character_id());
    drop(handler);
    // We don't have direct access to character_store here, so just return the raw id as name
    (sid_str.to_string(), char_id.map(|c| c.to_string()).unwrap_or_default())
}

/// Check if the current session has a required efun permission.
/// Returns Ok(()) if allowed, Err(LuaError) if denied.
/// On denial, writes an audit log entry.
fn check_efun_permission(
    efun_name: &str,
    perm_config: &PermissionConfig,
    sh: &Arc<RwLock<SessionHandler>>,
    game_logger: &Arc<GameLogger>,
) -> LuaResult<()> {
    if let Some(required) = perm_config.efuns.get(efun_name) {
        let current_sid = get_current_session();
        let allowed = current_sid
            .as_deref()
            .and_then(|s| s.parse::<SessionId>().ok())
            .map(|sid| sh.read().unwrap().has_permission(&sid, required))
            .unwrap_or(false);
        if !allowed {
            let (sid_str, char_name) = resolve_session_char(current_sid.as_deref(), sh);
            game_logger.audit(AuditEntry {
                session_id: &sid_str,
                char_name:  &char_name,
                action:     efun_name,
                success:    false,
                reason:     Some("permission denied"),
                extra:      Some(serde_json::json!({"required": required})),
            });
            return Err(LuaError::RuntimeError(
                format!("Permission denied: '{}' requires '{}'", efun_name, required)
            ));
        }
    }
    Ok(())
}

/// Register all efuns into the Lua global table
pub fn register_all(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    register_io_efuns(lua, ctx)?;
    register_session_efuns(lua, ctx)?;
    register_account_efuns(lua, ctx)?;
    register_character_efuns(lua, ctx)?;
    register_utility_efuns(lua, ctx)?;
    register_hot_reload_efuns(lua, ctx)?;
    register_object_state_efuns(lua, ctx)?;
    register_timer_efuns(lua, ctx)?;
    register_permission_efuns(lua, ctx)?;
    register_observability_efuns(lua, ctx)?;
    Ok(())
}

fn register_io_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // send(session_id, text)
    {
        let sh = ctx.session_handler.clone();
        let send_fn = lua.create_function(move |_, (session_id, text): (String, String)| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            let handler = sh.read().unwrap();
            let session = handler.get(&id)
                .ok_or_else(|| LuaError::RuntimeError(format!("Session not found: {}", session_id)))?;
            let _ = session.output_tx.try_send(SessionOutput::Text(text));
            Ok(())
        })?;
        globals.set("send", send_fn)?;
    }

    // send_prompt(session_id, text) — send without trailing newline (for input prompts)
    {
        let sh = ctx.session_handler.clone();
        let prompt_fn = lua.create_function(move |_, (session_id, text): (String, String)| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            let handler = sh.read().unwrap();
            if let Some(session) = handler.get(&id) {
                // Prompt text sent as raw — no trailing CRLF added by send_text
                let _ = session.output_tx.try_send(SessionOutput::Raw(text.into_bytes()));
            }
            Ok(())
        })?;
        globals.set("send_prompt", prompt_fn)?;
    }

    {
        let sh = ctx.session_handler.clone();
        let perm_config = ctx.permission_config.clone();
        let gl = ctx.game_logger.clone();
        let broadcast_fn = lua.create_function(move |_, text: String| {
            check_efun_permission("broadcast", &perm_config, &sh, &gl)?;
            sh.read().unwrap().broadcast(&text);
            Ok(())
        })?;
        globals.set("broadcast", broadcast_fn)?;
    }

    // disconnect(session_id)
    {
        let sh = ctx.session_handler.clone();
        let perm_config = ctx.permission_config.clone();
        let gl = ctx.game_logger.clone();
        let disconnect_fn = lua.create_function(move |_, session_id: String| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            // Only gate disconnecting OTHER sessions
            let current = get_current_session();
            let is_self = current.as_deref() == Some(&session_id);
            if !is_self {
                check_efun_permission("disconnect", &perm_config, &sh, &gl)?;
            }
            let handler = sh.read().unwrap();
            if let Some(session) = handler.get(&id) {
                let _ = session.output_tx.try_send(SessionOutput::Disconnect);
            }
            Ok(())
        })?;
        globals.set("disconnect", disconnect_fn)?;
    }

    // send_gmcp(session_id, package, data_table)
    {
        let sh = ctx.session_handler.clone();
        let gmcp_fn = lua.create_function(move |lua, (session_id, package, data): (String, String, LuaValue)| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            let json: JsonValue = lua_to_json(lua, &data)?;
            let handler = sh.read().unwrap();
            if let Some(session) = handler.get(&id) {
                let _ = session.output_tx.try_send(SessionOutput::Gmcp {
                    package,
                    data: json,
                });
            }
            Ok(())
        })?;
        globals.set("send_gmcp", gmcp_fn)?;
    }

    // start_echo(session_id)
    {
        let sh = ctx.session_handler.clone();
        let echo_fn = lua.create_function(move |_, session_id: String| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            let handler = sh.read().unwrap();
            if let Some(session) = handler.get(&id) {
                let _ = session.output_tx.try_send(SessionOutput::StartEcho);
            }
            Ok(())
        })?;
        globals.set("start_echo", echo_fn)?;
    }

    // stop_echo(session_id)
    {
        let sh = ctx.session_handler.clone();
        let echo_fn = lua.create_function(move |_, session_id: String| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            let handler = sh.read().unwrap();
            if let Some(session) = handler.get(&id) {
                let _ = session.output_tx.try_send(SessionOutput::StopEcho);
            }
            Ok(())
        })?;
        globals.set("stop_echo", echo_fn)?;
    }

    // File I/O efuns (jailed to mudlib root)
    super::efuns_io::register_io_file_efuns(
        lua,
        &ctx.mudlib_path,
        ctx.permission_config.clone(),
        ctx.session_handler.clone(),
    )?;

    Ok(())
}

fn register_session_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // this_session() -> string|nil
    let this_session_fn = lua.create_function(|_, ()| {
        Ok(get_current_session())
    })?;
    globals.set("this_session", this_session_fn)?;

    // get_session(session_id) -> table|nil
    {
        let sh = ctx.session_handler.clone();
        let get_session_fn = lua.create_function(move |lua, session_id: String| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            let handler = sh.read().unwrap();
            match handler.get(&id) {
                None => Ok(LuaValue::Nil),
                Some(s) => {
                    let t = lua.create_table()?;
                    t.set("id", s.id.to_string())?;
                    t.set("protocol", s.protocol.clone())?;
                    t.set("address", s.address.to_string())?;
                    t.set("state", s.state.name())?;
                    if let Some(aid) = s.state.account_id() {
                        t.set("account_id", aid)?;
                    }
                    if let Some(cid) = s.state.character_id() {
                        t.set("character_id", cid)?;
                    }
                    if let Some(w) = s.capabilities.window_width {
                        t.set("window_width", w)?;
                    }
                    if let Some(h) = s.capabilities.window_height {
                        t.set("window_height", h)?;
                    }
                    if let Some(ref ttype) = s.capabilities.terminal_type {
                        t.set("terminal_type", ttype.clone())?;
                    }
                    t.set("gmcp_supported", s.capabilities.gmcp_supported)?;
                    if !s.capabilities.gmcp_packages.is_empty() {
                        let pkgs = lua.create_table()?;
                        for (i, pkg) in s.capabilities.gmcp_packages.iter().enumerate() {
                            pkgs.set(i + 1, pkg.clone())?;
                        }
                        t.set("gmcp_packages", pkgs)?;
                    }
                    Ok(LuaValue::Table(t))
                }
            }
        })?;
        globals.set("get_session", get_session_fn)?;
    }

    // all_sessions() -> table (array of session_id strings)
    {
        let sh = ctx.session_handler.clone();
        let all_fn = lua.create_function(move |lua, ()| {
            let handler = sh.read().unwrap();
            let t = lua.create_table()?;
            for (i, id) in handler.all_ids().iter().enumerate() {
                t.set(i + 1, id.to_string())?;
            }
            Ok(t)
        })?;
        globals.set("all_sessions", all_fn)?;
    }

    // set_session_state(session_id, state_name)
    {
        let sh = ctx.session_handler.clone();
        let set_state_fn = lua.create_function(move |_, (session_id, state): (String, String)| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            sh.write().unwrap()
                .set_state_by_name(&id, &state)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))
        })?;
        globals.set("set_session_state", set_state_fn)?;
    }

    // authenticate_session(session_id, account_id)
    {
        let sh = ctx.session_handler.clone();
        let auth_fn = lua.create_function(move |_, (session_id, account_id): (String, i64)| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            let kicked = sh.write().unwrap()
                .authenticate(&id, account_id)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            Ok(kicked.map(|k| k.to_string()))
        })?;
        globals.set("authenticate_session", auth_fn)?;
    }

    // enter_game_session(session_id, account_id, character_id)
    {
        let sh = ctx.session_handler.clone();
        let account_store = ctx.account_store.clone();
        let role_store = ctx.role_store.clone();
        let enter_fn = lua.create_function(move |_, (session_id, account_id, character_id): (String, i64, i64)| {
            let id: SessionId = session_id.parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {}", e)))?;
            let is_admin = account_store.find_by_id(account_id)
                .ok()
                .flatten()
                .map(|a| a.is_admin)
                .unwrap_or(false);
            let perms = role_store.get_permissions_for_character(character_id)
                .unwrap_or_default();
            sh.write().unwrap()
                .enter_game(&id, account_id, character_id, perms, is_admin)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))
        })?;
        globals.set("enter_game_session", enter_fn)?;
    }

    Ok(())
}

fn register_account_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // authenticate(username, password) -> table|nil
    {
        let store = ctx.account_store.clone();
        let auth_fn = lua.create_function(move |lua, (username, password): (String, String)| {
            match store.authenticate(&username, &password) {
                Ok(account) => {
                    let json = account.to_lua_table();
                    Ok(json_to_lua(lua, &json)?)
                }
                Err(_) => Ok(LuaValue::Nil),
            }
        })?;
        globals.set("authenticate", auth_fn)?;
    }

    // create_account(username, password) -> table|nil
    {
        let store = ctx.account_store.clone();
        let create_fn = lua.create_function(move |lua, (username, password): (String, String)| {
            match store.create(&username, &password) {
                Ok(account) => {
                    let json = account.to_lua_table();
                    Ok(json_to_lua(lua, &json)?)
                }
                Err(e) => {
                    tracing::warn!("create_account failed: {}", e);
                    Ok(LuaValue::Nil)
                }
            }
        })?;
        globals.set("create_account", create_fn)?;
    }

    // get_account(id) -> table|nil
    {
        let store = ctx.account_store.clone();
        let get_fn = lua.create_function(move |lua, id: i64| {
            match store.find_by_id(id) {
                Ok(Some(account)) => {
                    let json = account.to_lua_table();
                    Ok(json_to_lua(lua, &json)?)
                }
                _ => Ok(LuaValue::Nil),
            }
        })?;
        globals.set("get_account", get_fn)?;
    }
    // set_admin(account_id, is_admin) -> bool
    {
        let store = ctx.account_store.clone();
        let set_admin_fn = lua.create_function(move |_, (account_id, is_admin): (i64, bool)| {
            match store.set_admin(account_id, is_admin) {
                Ok(()) => Ok(true),
                Err(e) => {
                    tracing::warn!("set_admin failed for account {}: {}", account_id, e);
                    Ok(false)
                }
            }
        })?;
        globals.set("set_admin", set_admin_fn)?;
    }

    Ok(())
}

fn register_character_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // create_character(account_id, name) -> table|nil
    {
        let store = ctx.character_store.clone();
        let create_fn = lua.create_function(move |lua, (account_id, name): (i64, String)| {
            match store.create(account_id, &name) {
                Ok(char) => {
                    let json = char.to_lua_table();
                    Ok(json_to_lua(lua, &json)?)
                }
                Err(e) => {
                    tracing::warn!("create_character failed: {}", e);
                    Ok(LuaValue::Nil)
                }
            }
        })?;
        globals.set("create_character", create_fn)?;
    }

    // get_characters(account_id) -> table (array)
    {
        let store = ctx.character_store.clone();
        let get_fn = lua.create_function(move |lua, account_id: i64| {
            let chars = store.find_by_account(account_id).unwrap_or_default();
            let t = lua.create_table()?;
            for (i, c) in chars.iter().enumerate() {
                let json = c.to_lua_table();
                t.set(i + 1, json_to_lua(lua, &json)?)?;
            }
            Ok(t)
        })?;
        globals.set("get_characters", get_fn)?;
    }

    // get_character(id) -> table|nil
    {
        let store = ctx.character_store.clone();
        let get_fn = lua.create_function(move |lua, id: i64| {
            match store.find_by_id(id) {
                Ok(Some(c)) => {
                    let json = c.to_lua_table();
                    Ok(json_to_lua(lua, &json)?)
                }
                _ => Ok(LuaValue::Nil),
            }
        })?;
        globals.set("get_character", get_fn)?;
    }

    // save_character_data(char_id, data_table) -> bool
    // Serializes a Lua table to JSON and stores it in the character's data column.
    {
        let store = ctx.character_store.clone();
        let save_fn = lua.create_function(move |lua, (char_id, data): (i64, LuaValue)| {
            let json = lua_to_json(lua, &data)?;
            let json_str = serde_json::to_string(&json)
                .map_err(|e| mlua::Error::external(e))?;
            match store.save_data(char_id, &json_str) {
                Ok(()) => Ok(true),
                Err(e) => {
                    tracing::warn!("save_character_data failed for char {}: {}", char_id, e);
                    Ok(false)
                }
            }
        })?;
        globals.set("save_character_data", save_fn)?;
    }

    // load_character_data(char_id) -> table|nil
    // Loads the character's data column from DB and deserializes from JSON to a Lua table.
    {
        let store = ctx.character_store.clone();
        let load_fn = lua.create_function(move |lua, char_id: i64| {
            match store.load_data(char_id) {
                Ok(Some(json_str)) => {
                    let json: JsonValue = serde_json::from_str(&json_str)
                        .unwrap_or(JsonValue::Object(serde_json::Map::new()));
                    Ok(json_to_lua(lua, &json)?)
                }
                Ok(None) => Ok(LuaValue::Nil),
                Err(e) => {
                    tracing::warn!("load_character_data failed for char {}: {}", char_id, e);
                    Ok(LuaValue::Nil)
                }
            }
        })?;
        globals.set("load_character_data", load_fn)?;
    }

    Ok(())
}

fn register_utility_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // log(level, message)
    let log_fn = lua.create_function(|_, (level, message): (String, String)| {
        match level.as_str() {
            "trace" => tracing::trace!("[Lua] {}", message),
            "debug" => tracing::debug!("[Lua] {}", message),
            "info"  => tracing::info!("[Lua] {}", message),
            "warn"  => tracing::warn!("[Lua] {}", message),
            "error" => tracing::error!("[Lua] {}", message),
            _       => tracing::info!("[Lua:{}] {}", level, message),
        }
        Ok(())
    })?;
    globals.set("log", log_fn)?;

    // time() -> number (Unix timestamp)
    let time_fn = lua.create_function(|_, ()| {
        Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as f64)
    })?;
    globals.set("time", time_fn)?;

    // config(key) -> any
    {
        let cfg = ctx.server_config.clone();
        let config_fn = lua.create_function(move |lua, key: String| {
            match key.as_str() {
                "game.name" => Ok(LuaValue::String(lua.create_string(&cfg.game.name)?)),
                "game.mudlib_path" => Ok(LuaValue::String(lua.create_string(&cfg.game.mudlib_path)?)),
                "game.game_path" => {
                    let path = cfg.game.game_path.as_deref().unwrap_or("./game");
                    Ok(LuaValue::String(lua.create_string(path)?))
                }
                "game.command_paths" => {
                    let default = vec!["cmds".to_string()];
                    let paths = cfg.game.command_paths.as_ref().unwrap_or(&default);
                    let t = lua.create_table()?;
                    for (i, p) in paths.iter().enumerate() {
                        t.set(i + 1, p.as_str())?;
                    }
                    Ok(LuaValue::Table(t))
                }
                "game.start_room" => {
                    match &cfg.game.start_room {
                        Some(room) => Ok(LuaValue::String(lua.create_string(room)?)),
                        None => Ok(LuaValue::Nil),
                    }
                }
                "accounts.max_characters_per_account" =>
                    Ok(LuaValue::Integer(cfg.accounts.max_characters_per_account as i64)),
                "accounts.allow_creation" =>
                    Ok(LuaValue::Boolean(cfg.accounts.allow_creation)),
                "sessions.multisession_mode" =>
                    Ok(LuaValue::String(lua.create_string("single")?)),
                "game.area_reset_seconds" => {
                    let val = cfg.game.area_reset_seconds.unwrap_or(900);
                    Ok(LuaValue::Integer(val as i64))
                }
                "game.autosave_seconds" => {
                    let val = cfg.game.autosave_seconds.unwrap_or(300);
                    Ok(LuaValue::Integer(val as i64))
                }
                _ => Ok(LuaValue::Nil),
            }
        })?;
        globals.set("config", config_fn)?;
    }
    // list_dir(relative_path) -> table of filenames (without .lua extension)
    // Lists .lua files in a directory relative to mudlib/ and game/ paths.
    // e.g. list_dir("cmds") returns {"north", "look", "say", ...}
    {
        let cfg = ctx.server_config.clone();
        let list_dir_fn = lua.create_function(move |lua, rel_path: String| {
            let t = lua.create_table()?;
            let mut idx = 1;
            let mut seen = std::collections::HashSet::new();

            // Search both game/ and mudlib/ directories
            let game_path = cfg.game.game_path.as_deref().unwrap_or("./game");
            let mudlib_path = &cfg.game.mudlib_path;
            let search_dirs = vec![game_path.to_string(), mudlib_path.clone()];

            for base in &search_dirs {
                let dir = std::path::PathBuf::from(base).join(&rel_path);
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) == Some("lua") {
                            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                let name = stem.to_string();
                                if seen.insert(name.clone()) {
                                    t.set(idx, name)?;
                                    idx += 1;
                                }
                            }
                        }
                    }
                }
            }

            Ok(LuaValue::Table(t))
        })?;
        globals.set("list_dir", list_dir_fn)?;
    }

    Ok(())
}

fn register_hot_reload_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    let persistent = lua.create_table()?;
    globals.set("_persistent_store", persistent)?;

    // set_persistent(key, value)
    let set_fn = lua.create_function(|lua, (key, value): (String, LuaValue)| {
        let store: LuaTable = lua.globals().get("_persistent_store")?;
        store.set(key, value)?;
        Ok(())
    })?;
    globals.set("set_persistent", set_fn)?;

    // get_persistent(key) -> any
    let get_fn = lua.create_function(|lua, key: String| {
        let store: LuaTable = lua.globals().get("_persistent_store")?;
        store.get::<LuaValue>(key)
    })?;
    globals.set("get_persistent", get_fn)?;

    // reload(module_name) — hot-reload a Lua module; admin-only in practice
    if let Some(tx) = &ctx.cmd_tx {
        let tx = tx.clone();
        let reload_fn = lua.create_function(move |_, module_name: String| {
            let _ = tx.send(LuaCommand::Reload { module_name });
            Ok(())
        })?;
        globals.set("reload", reload_fn)?;
    }

    Ok(())
}

fn register_object_state_efuns(lua: &Lua, _ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    let object_state = lua.create_table()?;
    globals.set("_object_state_store", object_state)?;

    // set_object_state(object_id, key, value)
    let set_fn = lua.create_function(|lua, (object_id, key, value): (String, String, LuaValue)| {
        let store: LuaTable = lua.globals().get("_object_state_store")?;
        let object_table: LuaTable = match store.get::<LuaTable>(object_id.as_str()) {
            Ok(t) => t,
            Err(_) => {
                let t = lua.create_table()?;
                store.set(object_id.as_str(), t.clone())?;
                t
            }
        };
        object_table.set(key, value)?;
        Ok(())
    })?;
    globals.set("set_object_state", set_fn)?;

    // get_object_state(object_id, key) -> value|nil
    let get_fn = lua.create_function(|lua, (object_id, key): (String, String)| {
        let store: LuaTable = lua.globals().get("_object_state_store")?;
        match store.get::<LuaTable>(object_id.as_str()) {
            Ok(object_table) => object_table.get::<LuaValue>(key),
            Err(_) => Ok(LuaValue::Nil),
        }
    })?;
    globals.set("get_object_state", get_fn)?;

    // get_all_object_state(object_id) -> table|nil
    let get_all_fn = lua.create_function(|lua, object_id: String| {
        let store: LuaTable = lua.globals().get("_object_state_store")?;
        store.get::<LuaValue>(object_id.as_str())
    })?;
    globals.set("get_all_object_state", get_all_fn)?;

    // clear_object_state(object_id)
    let clear_fn = lua.create_function(|lua, object_id: String| {
        let store: LuaTable = lua.globals().get("_object_state_store")?;
        store.set(object_id.as_str(), LuaValue::Nil)?;
        Ok(())
    })?;
    globals.set("clear_object_state", clear_fn)?;

    Ok(())
}

fn register_timer_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // Timer registry: maps timer ID → AbortHandle for cancellation
    let timer_registry: Arc<Mutex<HashMap<String, tokio::task::AbortHandle>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // We need a Tokio runtime handle to spawn async tasks from the sync Lua thread.
    // Use a Handle that was captured before entering the blocking thread.
    // IMPORTANT: We need to get the Tokio handle. Since the Lua thread is spawned
    // from within a Tokio context, we can capture the handle before spawning.
    // However, the Lua thread runs via std::thread::spawn (not tokio::spawn),
    // so we need to pass a runtime handle through EfunContext.
    //
    // For now, we'll create a small dedicated runtime for timers.
    let timer_rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_time()
            .build()
            .expect("Failed to create timer runtime")
    );

    // schedule_timer(id, delay_seconds) — one-shot timer
    if let Some(tx) = &ctx.cmd_tx {
        let tx = tx.clone();
        let registry = timer_registry.clone();
        let rt = timer_rt.clone();
        let schedule_fn = lua.create_function(move |_, (id, delay): (String, f64)| {
            let tx = tx.clone();
            let id_clone = id.clone();
            let registry_inner = registry.clone();

            let handle = rt.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
                let _ = tx.send(LuaCommand::TimerFired { id: id_clone.clone() });
                // Clean up from registry after firing
                if let Ok(mut reg) = registry_inner.lock() {
                    reg.remove(&id_clone);
                }
            });

            if let Ok(mut reg) = registry.lock() {
                reg.insert(id, handle.abort_handle());
            }
            Ok(())
        })?;
        globals.set("schedule_timer", schedule_fn)?;
    }

    // schedule_repeating(id, interval_seconds) — repeating timer
    if let Some(tx) = &ctx.cmd_tx {
        let tx = tx.clone();
        let registry = timer_registry.clone();
        let rt = timer_rt.clone();
        let schedule_fn = lua.create_function(move |_, (id, interval): (String, f64)| {
            let tx = tx.clone();
            let id_clone = id.clone();

            let handle = rt.spawn(async move {
                let mut ticker = tokio::time::interval(std::time::Duration::from_secs_f64(interval));
                ticker.tick().await; // first tick fires immediately, skip it
                loop {
                    ticker.tick().await;
                    if tx.send(LuaCommand::TimerFired { id: id_clone.clone() }).is_err() {
                        break; // channel closed
                    }
                }
            });

            if let Ok(mut reg) = registry.lock() {
                // Cancel any existing timer with the same ID
                if let Some(old) = reg.remove(&id) {
                    old.abort();
                }
                reg.insert(id, handle.abort_handle());
            }
            Ok(())
        })?;
        globals.set("schedule_repeating", schedule_fn)?;
    }

    // cancel_timer(id) — cancel a scheduled timer
    {
        let registry = timer_registry.clone();
        let cancel_fn = lua.create_function(move |_, id: String| {
            if let Ok(mut reg) = registry.lock() {
                if let Some(handle) = reg.remove(&id) {
                    handle.abort();
                    Ok(true)
                } else {
                    Ok(false)
                }
            } else {
                Ok(false)
            }
        })?;
        globals.set("cancel_timer", cancel_fn)?;
    }

    Ok(())
}

fn register_permission_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // has_permission(session_id, perm) -> bool
    {
        let sh = ctx.session_handler.clone();
        let fn_ = lua.create_function(move |_, (session_id, perm): (String, String)| {
            let id: SessionId = match session_id.parse() {
                Ok(id) => id,
                Err(_) => return Ok(false),
            };
            Ok(sh.read().unwrap().has_permission(&id, &perm))
        })?;
        globals.set("has_permission", fn_)?;
    }

    // refresh_permissions(session_id) -> bool
    {
        let sh = ctx.session_handler.clone();
        let account_store = ctx.account_store.clone();
        let role_store = ctx.role_store.clone();
        let fn_ = lua.create_function(move |_, session_id: String| {
            let id: SessionId = match session_id.parse() {
                Ok(id) => id,
                Err(_) => return Ok(false),
            };
            let character_id = {
                let h = sh.read().unwrap();
                h.get(&id).and_then(|s| s.state.character_id())
            };
            let account_id = {
                let h = sh.read().unwrap();
                h.get(&id).and_then(|s| s.state.account_id())
            };
            let (Some(character_id), Some(account_id)) = (character_id, account_id) else {
                return Ok(false);
            };
            let is_admin = account_store.find_by_id(account_id)
                .ok().flatten().map(|a| a.is_admin).unwrap_or(false);
            let perms = role_store.get_permissions_for_character(character_id)
                .unwrap_or_default();
            sh.write().unwrap().set_permissions(&id, perms, is_admin)
                .map(|_| true)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))
        })?;
        globals.set("refresh_permissions", fn_)?;
    }

    // create_role(name) -> table|nil
    {
        let store = ctx.role_store.clone();
        let fn_ = lua.create_function(move |lua, name: String| {
            match store.create_role(&name) {
                Ok(role) => {
                    let json = role.to_lua_table();
                    Ok(json_to_lua(lua, &json)?)
                }
                Err(e) => {
                    tracing::warn!("create_role failed: {}", e);
                    Ok(LuaValue::Nil)
                }
            }
        })?;
        globals.set("create_role", fn_)?;
    }

    // delete_role(name) -> bool
    {
        let store = ctx.role_store.clone();
        let fn_ = lua.create_function(move |_, name: String| {
            match store.find_role_by_name(&name) {
                Ok(Some(role)) => Ok(store.delete_role(role.id).is_ok()),
                _ => Ok(false),
            }
        })?;
        globals.set("delete_role", fn_)?;
    }

    // list_roles() -> array of {id, name}
    {
        let store = ctx.role_store.clone();
        let fn_ = lua.create_function(move |lua, ()| {
            let roles = store.list_roles().unwrap_or_default();
            let t = lua.create_table()?;
            for (i, r) in roles.iter().enumerate() {
                let json = r.to_lua_table();
                t.set(i + 1, json_to_lua(lua, &json)?)?;
            }
            Ok(t)
        })?;
        globals.set("list_roles", fn_)?;
    }

    // assign_role(character_id, role_name) -> bool
    {
        let store = ctx.role_store.clone();
        let sh_ref = ctx.session_handler.clone();
        let account_store = ctx.account_store.clone();
        let fn_ = lua.create_function(move |_, (character_id, role_name): (i64, String)| {
            let role = match store.find_role_by_name(&role_name) {
                Ok(Some(r)) => r,
                _ => return Ok(false),
            };
            let ok = store.assign_role(character_id, role.id).is_ok();
            if ok {
                let sids: Vec<_> = {
                    let h = sh_ref.read().unwrap();
                    h.all_ids().into_iter().filter(|sid| {
                        h.get(sid).and_then(|s| s.state.character_id()) == Some(character_id)
                    }).collect()
                };
                for sid in sids {
                    let account_id = sh_ref.read().unwrap().get(&sid).and_then(|s| s.state.account_id());
                    if let Some(aid) = account_id {
                        let is_admin = account_store.find_by_id(aid).ok().flatten().map(|a| a.is_admin).unwrap_or(false);
                        let perms = store.get_permissions_for_character(character_id).unwrap_or_default();
                        let _ = sh_ref.write().unwrap().set_permissions(&sid, perms, is_admin);
                    }
                }
            }
            Ok(ok)
        })?;
        globals.set("assign_role", fn_)?;
    }

    // revoke_role(character_id, role_name) -> bool
    {
        let store = ctx.role_store.clone();
        let sh_ref = ctx.session_handler.clone();
        let account_store = ctx.account_store.clone();
        let fn_ = lua.create_function(move |_, (character_id, role_name): (i64, String)| {
            let role = match store.find_role_by_name(&role_name) {
                Ok(Some(r)) => r,
                _ => return Ok(false),
            };
            let ok = store.revoke_role(character_id, role.id).is_ok();
            if ok {
                let sids: Vec<_> = {
                    let h = sh_ref.read().unwrap();
                    h.all_ids().into_iter().filter(|sid| {
                        h.get(sid).and_then(|s| s.state.character_id()) == Some(character_id)
                    }).collect()
                };
                for sid in sids {
                    let account_id = sh_ref.read().unwrap().get(&sid).and_then(|s| s.state.account_id());
                    if let Some(aid) = account_id {
                        let is_admin = account_store.find_by_id(aid).ok().flatten().map(|a| a.is_admin).unwrap_or(false);
                        let perms = store.get_permissions_for_character(character_id).unwrap_or_default();
                        let _ = sh_ref.write().unwrap().set_permissions(&sid, perms, is_admin);
                    }
                }
            }
            Ok(ok)
        })?;
        globals.set("revoke_role", fn_)?;
    }

    // get_roles(character_id) -> array of role name strings
    {
        let store = ctx.role_store.clone();
        let fn_ = lua.create_function(move |lua, character_id: i64| {
            let roles = store.get_roles_for_character(character_id).unwrap_or_default();
            let t = lua.create_table()?;
            for (i, r) in roles.iter().enumerate() {
                t.set(i + 1, r.name.clone())?;
            }
            Ok(t)
        })?;
        globals.set("get_roles", fn_)?;
    }

    // grant_permission(role_name, perm_string) -> bool
    {
        let store = ctx.role_store.clone();
        let fn_ = lua.create_function(move |_, (role_name, perm): (String, String)| {
            let role = match store.find_role_by_name(&role_name) {
                Ok(Some(r)) => r,
                _ => return Ok(false),
            };
            Ok(store.grant_permission(role.id, &perm).is_ok())
        })?;
        globals.set("grant_permission", fn_)?;
    }

    // revoke_permission(role_name, perm_string) -> bool
    {
        let store = ctx.role_store.clone();
        let fn_ = lua.create_function(move |_, (role_name, perm): (String, String)| {
            let role = match store.find_role_by_name(&role_name) {
                Ok(Some(r)) => r,
                _ => return Ok(false),
            };
            Ok(store.revoke_permission(role.id, &perm).is_ok())
        })?;
        globals.set("revoke_permission", fn_)?;
    }

    // get_permissions(role_name) -> array of permission strings
    {
        let store = ctx.role_store.clone();
        let fn_ = lua.create_function(move |lua, role_name: String| {
            let role = match store.find_role_by_name(&role_name) {
                Ok(Some(r)) => r,
                _ => {
                    let t = lua.create_table()?;
                    return Ok(t);
                }
            };
            let perms = store.get_permissions_for_role(role.id).unwrap_or_default();
            let t = lua.create_table()?;
            for (i, p) in perms.iter().enumerate() {
                t.set(i + 1, p.clone())?;
            }
            Ok(t)
        })?;
        globals.set("get_permissions", fn_)?;
    }

    Ok(())
}

fn register_observability_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // server_info() -> table
    {
        let cfg = ctx.server_config.clone();
        let started_at_utc = ctx.started_at_utc.clone();
        let started_at = ctx.started_at;
        let fn_ = lua.create_function(move |lua, ()| {
            let uptime_secs = started_at.elapsed().as_secs_f64();
            let t = lua.create_table()?;
            t.set("version",     env!("CARGO_PKG_VERSION"))?;
            t.set("name",        cfg.game.name.clone())?;
            t.set("started_at",  started_at_utc.clone())?;
            t.set("uptime_secs", uptime_secs)?;
            Ok(t)
        })?;
        globals.set("server_info", fn_)?;
    }

    // journal_write(level, message, meta?) -> bool
    {
        let gl = ctx.game_logger.clone();
        let sh = ctx.session_handler.clone();
        let fn_ = lua.create_function(move |_, (level, message, meta): (String, String, Option<String>)| {
            let (sid, char_name) = resolve_session_char(get_current_session().as_deref(), &sh);
            let meta_val = meta
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .unwrap_or_else(|| serde_json::json!({"sid": &sid, "char": &char_name}));
            let source = get_current_session()
                .map(|s| {
                    let len = s.len().min(8);
                    format!("lua:{}", &s[..len])
                })
                .unwrap_or_else(|| "lua".to_string());
            gl.journal(JournalEntry {
                level:   &level,
                source:  &source,
                message: &message,
                meta:    Some(meta_val),
            });
            Ok(true)
        })?;
        globals.set("journal_write", fn_)?;
    }

    // audit_write(action, success, reason?) -> bool
    {
        let gl = ctx.game_logger.clone();
        let sh = ctx.session_handler.clone();
        let fn_ = lua.create_function(move |_, (action, success, reason): (String, bool, Option<String>)| {
            let (sid, char_name) = resolve_session_char(get_current_session().as_deref(), &sh);
            gl.audit(AuditEntry {
                session_id: &sid,
                char_name:  &char_name,
                action:     &action,
                success,
                reason:     reason.as_deref(),
                extra:      None,
            });
            Ok(true)
        })?;
        globals.set("audit_write", fn_)?;
    }

    // journal_read(limit?, level?) -> array of strings
    {
        let gl = ctx.game_logger.clone();
        let perm_config = ctx.permission_config.clone();
        let sh = ctx.session_handler.clone();
        let fn_ = lua.create_function(move |lua, (limit, level): (Option<usize>, Option<String>)| {
            check_efun_permission("journal_read", &perm_config, &sh, &gl)?;
            let lines = gl.read_journal(limit.unwrap_or(20), level.as_deref());
            let t = lua.create_table()?;
            for (i, l) in lines.iter().enumerate() {
                t.set(i + 1, l.clone())?;
            }
            Ok(t)
        })?;
        globals.set("journal_read", fn_)?;
    }

    // audit_read(limit?) -> array of strings
    {
        let gl = ctx.game_logger.clone();
        let perm_config = ctx.permission_config.clone();
        let sh = ctx.session_handler.clone();
        let fn_ = lua.create_function(move |lua, limit: Option<usize>| {
            check_efun_permission("audit_read", &perm_config, &sh, &gl)?;
            let lines = gl.read_audit(limit.unwrap_or(20));
            let t = lua.create_table()?;
            for (i, l) in lines.iter().enumerate() {
                t.set(i + 1, l.clone())?;
            }
            Ok(t)
        })?;
        globals.set("audit_read", fn_)?;
    }

    // broadcast_to_perm(perm, msg) -> number (count of recipients)
    {
        let sh = ctx.session_handler.clone();
        let perm_config = ctx.permission_config.clone();
        let gl = ctx.game_logger.clone();
        let fn_ = lua.create_function(move |_, (perm, msg): (String, String)| {
            check_efun_permission("broadcast_to_perm", &perm_config, &sh, &gl)?;
            // Collect session IDs and senders first, then release lock, then send
            let targets: Vec<_> = {
                let handler = sh.read().unwrap();
                handler.all_ids().into_iter().filter_map(|sid| {
                    if handler.has_permission(&sid, &perm) {
                        handler.get(&sid).map(|s| (sid, s.output_tx.clone()))
                    } else {
                        None
                    }
                }).collect()
            };
            let count = targets.len();
            for (_sid, tx) in targets {
                let _ = tx.try_send(SessionOutput::Text(msg.clone()));
            }
            Ok(count)
        })?;
        globals.set("broadcast_to_perm", fn_)?;
    }

    // verify_file(path) -> (bool, string?)
    // Compiles a mudlib file WITHOUT executing it.
    {
        let mudlib_path = ctx.mudlib_path.clone();
        let perm_config = ctx.permission_config.clone();
        let sh = ctx.session_handler.clone();
        let gl = ctx.game_logger.clone();
        let fn_ = lua.create_function(move |lua, path: String| {
            check_efun_permission("verify_file", &perm_config, &sh, &gl)?;
            let resolved = match crate::core::scripting::sandbox::resolve_jailed_path(&mudlib_path, &path) {
                Ok(p) => p,
                Err(e) => return Ok((false, Some(format!("Path error: {}", e)))),
            };
            let code = match std::fs::read_to_string(&resolved) {
                Ok(c) => c,
                Err(e) => return Ok((false, Some(format!("Cannot read '{}': {}", path, e)))),
            };
            let chunk_name = format!("@{}", path);
            match lua.load(code.as_str()).set_name(&chunk_name).into_function() {
                Ok(_)  => Ok((true, None)),
                Err(e) => Ok((false, Some(e.to_string()))),
            }
        })?;
        globals.set("verify_file", fn_)?;
    }

    Ok(())
}


/// Convert a Lua value to serde_json::Value
pub fn lua_to_json(lua: &Lua, val: &LuaValue) -> LuaResult<JsonValue> {
    match val {
        LuaValue::Nil => Ok(JsonValue::Null),
        LuaValue::Boolean(b) => Ok(JsonValue::Bool(*b)),
        LuaValue::Integer(i) => Ok(JsonValue::Number((*i).into())),
        LuaValue::Number(n) => {
            let num = serde_json::Number::from_f64(*n)
                .unwrap_or_else(|| 0.into());
            Ok(JsonValue::Number(num))
        }
        LuaValue::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
        LuaValue::Table(t) => {
            // Check if it's array-like
            let len = t.len()? as usize;
            if len > 0 {
                let mut arr = Vec::new();
                for i in 1..=len {
                    let v: LuaValue = t.get(i)?;
                    arr.push(lua_to_json(lua, &v)?);
                }
                Ok(JsonValue::Array(arr))
            } else {
                let mut map = serde_json::Map::new();
                for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                    let (k, v) = pair?;
                    let key = match &k {
                        LuaValue::String(s) => s.to_str()?.to_string(),
                        LuaValue::Integer(i) => i.to_string(),
                        _ => continue,
                    };
                    map.insert(key, lua_to_json(lua, &v)?);
                }
                Ok(JsonValue::Object(map))
            }
        }
        _ => Ok(JsonValue::Null),
    }
}

/// Convert a serde_json::Value to Lua value
pub fn json_to_lua(lua: &Lua, val: &JsonValue) -> LuaResult<LuaValue> {
    match val {
        JsonValue::Null => Ok(LuaValue::Nil),
        JsonValue::Bool(b) => Ok(LuaValue::Boolean(*b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(LuaValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(LuaValue::Number(f))
            } else {
                Ok(LuaValue::Nil)
            }
        }
        JsonValue::String(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        JsonValue::Array(arr) => {
            let t = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(t))
        }
        JsonValue::Object(map) => {
            let t = lua.create_table()?;
            for (k, v) in map {
                t.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(t))
        }
    }
}
