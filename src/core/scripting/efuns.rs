use std::sync::{Arc, RwLock, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use mlua::prelude::*;
use serde_json::Value as JsonValue;
use tokio::sync::mpsc::UnboundedSender;
use crate::core::lock::RwLockExt;

use crate::config::server_config::ServerConfig;
use crate::config::permissions_config::PermissionConfig;
use crate::core::session::{SessionHandler, SessionOutput, SessionId};
use crate::core::scripting::engine::LuaCommand;
use crate::core::logging::{GameLogger, AuditEntry, JournalEntry};
use crate::domain::models::{DieselAccountStore, DieselCharacterStore, DieselDocumentStore};
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
    /// Shared control block for tracing and the debug adapter
    pub debug_state:        crate::core::scripting::debugger::SharedDebugState,
    /// Pool that runs Argon2 off the Lua thread. `None` disables the
    /// `authenticate` and `create_account` efuns entirely — see
    /// `register_auth_efuns`.
    pub auth_worker:        Option<crate::core::auth::AuthWorker>,
    /// Pool that runs Lua on worker threads. `None` disables the `compute`
    /// efun entirely — see `efuns_compute::register_compute_efuns`.
    pub compute:            Option<crate::core::compute::ComputeBridge>,
    /// The generic JSON document store behind the `db_*` efuns.
    pub document_store:     Arc<DieselDocumentStore>,
}

// The currently-active session ID for the Lua thread.
// Set by the engine before dispatching each event to Lua.
// This is a thread-local since the Lua VM runs on a single dedicated thread.
thread_local! {
    static CURRENT_SESSION: std::cell::RefCell<Option<String>> =
        std::cell::RefCell::new(None);

    /// Whether the engine is currently dispatching on its own behalf.
    ///
    /// Timer ticks, hot reloads and the initial mudlib load have no player
    /// behind them, so `CURRENT_SESSION` is `None` for all of them. That used
    /// to mean `check_efun_permission` failed closed: any gated efun called
    /// from a daemon tick was denied, silently, because nothing was there to
    /// see the error. This flag makes the alternative an explicit decision
    /// rather than something that falls out of an unset thread-local.
    static SYSTEM_DISPATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn set_current_session(id: Option<String>) {
    CURRENT_SESSION.with(|s| *s.borrow_mut() = id);
}

pub fn get_current_session() -> Option<String> {
    CURRENT_SESSION.with(|s| s.borrow().clone())
}

/// The label used for the engine itself in audit and journal entries.
pub const SYSTEM_ACTOR: &str = "system";

/// Whether the current dispatch is the engine acting on its own behalf.
pub fn is_system_dispatch() -> bool {
    SYSTEM_DISPATCH.with(std::cell::Cell::get)
}

/// Restores the previous dispatch identity when dropped, so a Lua error
/// unwinding out of a dispatch cannot leave the flag stuck on.
pub struct SystemDispatchGuard(bool);

impl Drop for SystemDispatchGuard {
    fn drop(&mut self) {
        SYSTEM_DISPATCH.with(|c| c.set(self.0));
    }
}

/// Mark everything until the returned guard drops as engine-internal.
///
/// Gated efuns are permitted for the duration. That is a real widening: any
/// code reachable from a timer tick, a reload, or mudlib load runs with the
/// driver's own authority. It is confined to code paths that only *authored*
/// Lua can reach — a player cannot register a ticker callback — which is the
/// same trust boundary as being able to write a room file at all.
#[must_use = "the identity is restored when the guard drops"]
pub fn enter_system_dispatch() -> SystemDispatchGuard {
    let previous = SYSTEM_DISPATCH.with(|c| c.replace(true));
    SystemDispatchGuard(previous)
}

/// Who to record as the actor for a log or audit entry.
pub fn current_actor() -> String {
    if let Some(sid) = get_current_session() {
        return sid;
    }
    if is_system_dispatch() {
        return SYSTEM_ACTOR.to_string();
    }
    "unknown".to_string()
}

/// Resolve (session_id_str, character_name) for audit entries.
/// Returns ("unknown", "") if the session isn't found.
fn resolve_session_char(
    sid: Option<&str>,
    sh: &Arc<RwLock<SessionHandler>>,
) -> (String, String) {
    let Some(sid_str) = sid else {
        // No session: either the engine acting for itself, or nothing at all.
        // Saying which is the whole point of the distinction.
        return (current_actor(), "".to_string());
    };
    let id: SessionId = match sid_str.parse() {
        Ok(id) => id,
        Err(_) => return (sid_str.to_string(), "".to_string()),
    };
    let handler = sh.read_recover();
    let char_id = handler.get(&id).and_then(|s| s.state.character_id());
    drop(handler);
    // We don't have direct access to character_store here, so just return the raw id as name
    (sid_str.to_string(), char_id.map(|c| c.to_string()).unwrap_or_default())
}

/// Check if the current session has a required efun permission.
/// Returns Ok(()) if allowed, Err(LuaError) if denied.
/// On denial, writes an audit log entry.
pub(crate) fn check_efun_permission(
    efun_name: &str,
    perm_config: &PermissionConfig,
    sh: &Arc<RwLock<SessionHandler>>,
    game_logger: &Arc<GameLogger>,
) -> LuaResult<()> {
    if let Some(required) = perm_config.efuns.get(efun_name) {
        // Engine-internal dispatch acts with the driver's own authority. There
        // is no session to check, and failing closed here is what made a
        // daemon's `write_file` on a tick fail silently — with no player
        // connected, nothing surfaced the error.
        if is_system_dispatch() {
            tracing::debug!("efun '{}' permitted for engine-internal dispatch", efun_name);
            return Ok(());
        }

        let current_sid = get_current_session();
        let allowed = current_sid
            .as_deref()
            .and_then(|s| s.parse::<SessionId>().ok())
            .map(|sid| sh.read_recover().has_permission(&sid, required))
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
    super::debugger::efuns::register_debug_efuns(lua, ctx)?;
    super::efuns_compute::register_compute_efuns(lua, ctx)?;
    super::efuns_document::register_document_efuns(lua, ctx)?;
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
            let handler = sh.read_recover();
            let session = handler.get(&id)
                .ok_or_else(|| LuaError::RuntimeError(format!("Session not found: {}", session_id)))?;
            session.try_send(SessionOutput::Text(text));
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
            let handler = sh.read_recover();
            if let Some(session) = handler.get(&id) {
                // Prompt text sent as raw — no trailing CRLF added by send_text
                session.try_send(SessionOutput::Raw(text.into_bytes()));
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
            let dropped = sh.read_recover().broadcast(&text);
            if dropped > 0 {
                // Each of those sessions gets a truncation marker of its own;
                // this is for the operator, who would otherwise have no idea a
                // broadcast did not reach everyone.
                tracing::warn!("broadcast did not reach {} session(s) — output channels full", dropped);
                gl.journal(JournalEntry {
                    level:   "warn",
                    source:  "broadcast",
                    message: "broadcast dropped for one or more sessions",
                    meta:    Some(serde_json::json!({"dropped": dropped})),
                });
            }
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
            let handler = sh.read_recover();
            if let Some(session) = handler.get(&id) {
                session.try_send(SessionOutput::Disconnect);
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
            let handler = sh.read_recover();
            if let Some(session) = handler.get(&id) {
                session.try_send(SessionOutput::Gmcp {
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
            let handler = sh.read_recover();
            if let Some(session) = handler.get(&id) {
                session.try_send(SessionOutput::StartEcho);
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
            let handler = sh.read_recover();
            if let Some(session) = handler.get(&id) {
                session.try_send(SessionOutput::StopEcho);
            }
            Ok(())
        })?;
        globals.set("stop_echo", echo_fn)?;
    }

    // File I/O efuns. Jailed to **two** roots — the mudlib and the game layer —
    // each checked against its own. A path may name one with a `game:` or
    // `mudlib:` prefix; unprefixed, a read searches game-then-mudlib and a write
    // stays in the mudlib. See `efuns_io::Roots`.
    let game_root = ctx
        .server_config
        .game
        .game_path
        .as_deref()
        .map(std::path::PathBuf::from);
    super::efuns_io::register_io_file_efuns(
        lua,
        &ctx.mudlib_path,
        game_root.as_deref(),
        ctx.permission_config.clone(),
        ctx.session_handler.clone(),
        ctx.debug_state.clone(),
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
            let handler = sh.read_recover();
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
                    t.set("dropped_output", s.dropped_output() as i64)?;
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
            let handler = sh.read_recover();
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
            sh.write_recover()
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
            let kicked = sh.write_recover()
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
            sh.write_recover()
                .enter_game(&id, account_id, character_id, perms, is_admin)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))
        })?;
        globals.set("enter_game_session", enter_fn)?;
    }

    Ok(())
}

fn register_account_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // authenticate(session_id, username, password)
    // create_account(session_id, username, password)
    //
    // Both are non-blocking and return nothing. Argon2 costs a few hundred
    // milliseconds, and running it here would stop the whole game for that
    // long — before authentication, so anyone with a socket could do it on
    // demand. The result arrives later as `on_auth_result`.
    register_auth_efuns(lua, ctx)?;

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

/// The two efuns that hash a password, both dispatched to [`crate::core::auth`].
///
/// They are registered together because they share the whole shape: take a
/// session, hand the work to the pool, and say nothing on success — the answer
/// comes back through `on_auth_result`. A refusal (queue full, address locked
/// out) is reported through the *same* hook rather than as a return value, so
/// the mudlib has exactly one place that finishes a login.
fn register_auth_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    let Some(worker) = ctx.auth_worker.clone() else {
        // No worker means no cmd_tx to answer on, which only happens in tests
        // that never log in. Registering nothing is better than registering a
        // version that blocks: a missing efun fails loudly at the call site.
        tracing::debug!("No auth worker configured — authenticate/create_account not registered");
        return Ok(());
    };

    for (name, kind) in [
        ("authenticate", crate::core::auth::AuthKind::Authenticate),
        ("create_account", crate::core::auth::AuthKind::CreateAccount),
    ] {
        let worker = worker.clone();
        let sh = ctx.session_handler.clone();
        let cmd_tx = ctx.cmd_tx.clone();
        let f = lua.create_function(
            move |_, (session_id, username, password): (String, String, String)| {
                let peer = session_id
                    .parse::<SessionId>()
                    .ok()
                    .and_then(|id| {
                        sh.read_recover()
                            .get(&id)
                            .map(|s| s.address.ip())
                    });

                if let Err(refused) = worker.submit(
                    session_id.clone(),
                    kind,
                    username,
                    password,
                    peer,
                ) {
                    tracing::info!("auth refused for session {}: {:?}", session_id, refused);
                    // Answer on the same path a worker would, so the mudlib
                    // never has to handle "the efun told me" and "the hook told
                    // me" as two different cases.
                    if let Some(tx) = &cmd_tx {
                        let _ = tx.send(LuaCommand::AuthResult {
                            session_id,
                            kind: kind.as_str(),
                            account: None,
                            error: Some(refused.message()),
                        });
                    }
                }
                Ok(())
            },
        )?;
        globals.set(name, f)?;
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

    // time() -> integer (Unix timestamp, whole seconds)
    //
    // An integer, not a float. On LuaJIT every number is a double and this made
    // no difference; on 5.3+ integers are a real subtype, and a float timestamp
    // renders as `1712345678.0` — which reaches players through
    // `event_d.lua`'s deferred timer ids and anything else that concatenates a
    // timestamp into a string.
    let time_fn = lua.create_function(|_, ()| {
        Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64)
    })?;
    globals.set("time", time_fn)?;

    // config(key) -> any
    //
    // A generic dotted-path reader over the whole `ServerConfig`, not a list of
    // keys someone remembered to add. It was an eighteen-key `match`, which
    // meant any setting the game layer wanted — a respawn room, a shop restock
    // interval — needed a Rust edit before Lua could see it, and that pressure
    // is why `death_d` had a game room hardcoded in the mudlib layer.
    //
    // `[game]` captures unknown keys into `GameConfig::extra`, so
    // `config("game.respawn_room")` works from a `server.toml` edit alone.
    // Defaults live in `CONFIG_DEFAULTS` rather than at each call site, because
    // a default repeated in two places is a default two places can disagree
    // about.
    {
        // Serialised once here, not per call: `config()` sits on the command
        // dispatch path via `game.command_paths`, and this runs at VM
        // construction where a few microseconds cost nothing.
        let snapshot = ctx.server_config.as_lookup_json();
        let config_fn = lua.create_function(move |lua, key: String| {
            let mut node = &snapshot;
            for part in key.split('.') {
                match node.get(part) {
                    Some(next) => node = next,
                    // An unknown key is `nil`, not an error. Lua reads config
                    // with `or <fallback>` throughout, and a raise would turn
                    // every typo into a dead daemon rather than a default.
                    None => return Ok(LuaValue::Nil),
                }
            }
            json_to_lua(lua, node)
        })?;
        globals.set("config", config_fn)?;
    }
    // `list_dir` deliberately does NOT live here. It used to — an unjailed
    // second copy registered after `register_io_efuns`, which silently
    // overwrote the permission-checked, path-jailed version in `efuns_io.rs`.
    // `list_dir("../../..")` escaped for as long as that was true, while
    // `file-access.md` and `sandboxing.md` both claimed traversal prevention
    // "for all file efuns". There is one implementation now, in `efuns_io.rs`,
    // and `tests/list_dir_jail.rs` reaches it the way game code does.

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

/// Recompute the permission cache for every playing session whose character is
/// in `characters`, or for all of them when it is `None`.
///
/// `has_permission` reads a per-session cache seeded at `enter_game_session`,
/// so anything that changes what a character may do has to say so or the change
/// does not reach anyone who is already online. `assign_role` and `revoke_role`
/// already did this; `grant_permission` and `revoke_permission` did not, which
/// made the two halves of the RBAC surface disagree — an admin who watched a
/// role assignment take effect immediately would reasonably expect editing the
/// role itself to behave the same way, and it silently did not.
///
/// Changing a role's contents resyncs *everyone*, because working out who holds
/// that role costs a query per session anyway and there is no reverse index.
/// This is an admin action measured in times-per-week, against a session list
/// measured in hundreds; `refresh_permissions` remains the explicit escape
/// hatch for anything this cannot see.
fn resync_permission_cache(
    sh: &Arc<std::sync::RwLock<SessionHandler>>,
    role_store: &Arc<DieselRoleStore>,
    account_store: &Arc<DieselAccountStore>,
    characters: Option<i64>,
) {
    let targets: Vec<(SessionId, i64, i64)> = {
        let h = sh.read_recover();
        h.all_ids()
            .into_iter()
            .filter_map(|sid| {
                let s = h.get(&sid)?;
                let cid = s.state.character_id()?;
                let aid = s.state.account_id()?;
                match characters {
                    Some(want) if want != cid => None,
                    _ => Some((sid, aid, cid)),
                }
            })
            .collect()
    };

    for (sid, account_id, character_id) in targets {
        let is_admin = account_store
            .find_by_id(account_id)
            .ok()
            .flatten()
            .map(|a| a.is_admin)
            .unwrap_or(false);
        let perms = role_store
            .get_permissions_for_character(character_id)
            .unwrap_or_default();
        let _ = sh.write_recover().set_permissions(&sid, perms, is_admin);
    }
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
            Ok(sh.read_recover().has_permission(&id, &perm))
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
                let h = sh.read_recover();
                h.get(&id).and_then(|s| s.state.character_id())
            };
            let account_id = {
                let h = sh.read_recover();
                h.get(&id).and_then(|s| s.state.account_id())
            };
            let (Some(character_id), Some(account_id)) = (character_id, account_id) else {
                return Ok(false);
            };
            let is_admin = account_store.find_by_id(account_id)
                .ok().flatten().map(|a| a.is_admin).unwrap_or(false);
            let perms = role_store.get_permissions_for_character(character_id)
                .unwrap_or_default();
            sh.write_recover().set_permissions(&id, perms, is_admin)
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
                resync_permission_cache(&sh_ref, &store, &account_store, Some(character_id));
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
                resync_permission_cache(&sh_ref, &store, &account_store, Some(character_id));
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
    //
    // Resyncs every playing session, because editing a role changes what
    // everyone holding it may do and there is no reverse index from role to
    // character. Without this, `assign_role` took effect immediately and
    // `grant_permission` did not — the same surface behaving two ways.
    {
        let store = ctx.role_store.clone();
        let sh_ref = ctx.session_handler.clone();
        let account_store = ctx.account_store.clone();
        let fn_ = lua.create_function(move |_, (role_name, perm): (String, String)| {
            let role = match store.find_role_by_name(&role_name) {
                Ok(Some(r)) => r,
                _ => return Ok(false),
            };
            let ok = store.grant_permission(role.id, &perm).is_ok();
            if ok {
                resync_permission_cache(&sh_ref, &store, &account_store, None);
            }
            Ok(ok)
        })?;
        globals.set("grant_permission", fn_)?;
    }

    // revoke_permission(role_name, perm_string) -> bool
    //
    // The direction that matters: a permission that outlives its revocation is
    // a security problem, not an inconvenience.
    {
        let store = ctx.role_store.clone();
        let sh_ref = ctx.session_handler.clone();
        let account_store = ctx.account_store.clone();
        let fn_ = lua.create_function(move |_, (role_name, perm): (String, String)| {
            let role = match store.find_role_by_name(&role_name) {
                Ok(Some(r)) => r,
                _ => return Ok(false),
            };
            let ok = store.revoke_permission(role.id, &perm).is_ok();
            if ok {
                resync_permission_cache(&sh_ref, &store, &account_store, None);
            }
            Ok(ok)
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

    // ─── Heap and GC visibility ──────────────────────────────────────────────
    //
    // Nothing measured any of this. There were zero `collectgarbage` calls
    // anywhere and no Rust-side GC configuration, so LuaJIT ran at its default
    // pause of 200 — the heap roughly doubles before a full cycle — against a
    // `lua_memory_mb = 64` ceiling. A live set nearing ~32 MB grows into that
    // ceiling, LuaJIT runs an emergency full collection before failing, and the
    // signature under pressure is *latency spikes first, catchable allocation
    // errors second*, surfacing in whatever code happened to allocate rather
    // than in the code responsible.
    //
    // These counters exist so that any later `setpause`/`setstepmul` change is
    // justified by a number rather than by intuition. **Do not tune GC
    // parameters without one.** Defaults are usually right, and tuning blind
    // makes things worse.
    let gc_full_count = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let gc_full_micros = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let gc_freed_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    // gc_collect() -> table  { freed_bytes, ms, heap_bytes }
    //
    // Runs a full collection and reports what it cost and what it recovered.
    // This is the instrument the heap drill uses: record the heap at boot, run
    // an hour of mob respawns / a walk into the virtual grid / several reloads,
    // and collect. The number should come back close to its baseline each time.
    // A monotonic climb across all three is the signature that object-state
    // leaks, uncached virtual rooms and closure retention on hot reload produce,
    // and it is the only way to tell those apart from an ordinary working set.
    {
        let count = gc_full_count.clone();
        let micros = gc_full_micros.clone();
        let freed = gc_freed_bytes.clone();
        let fn_ = lua.create_function(move |lua, ()| {
            use std::sync::atomic::Ordering;
            let before = lua.used_memory() as u64;
            let started = std::time::Instant::now();
            lua.gc_collect()?;
            // Twice: LuaJIT's incremental collector needs a second full cycle
            // to sweep what the first one finalised, and a single call
            // consistently under-reports what is actually reclaimable.
            lua.gc_collect()?;
            let elapsed = started.elapsed();
            let after = lua.used_memory() as u64;

            count.fetch_add(1, Ordering::Relaxed);
            micros.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
            let recovered = before.saturating_sub(after);
            freed.fetch_add(recovered, Ordering::Relaxed);

            let t = lua.create_table()?;
            t.set("freed_bytes", recovered as i64)?;
            t.set("ms", elapsed.as_secs_f64() * 1000.0)?;
            t.set("heap_bytes", after as i64)?;
            Ok(t)
        })?;
        globals.set("gc_collect", fn_)?;
    }

    // server_info() -> table
    {
        let cfg = ctx.server_config.clone();
        let started_at_utc = ctx.started_at_utc.clone();
        let started_at = ctx.started_at;
        let sh = ctx.session_handler.clone();
        let compute = ctx.compute.clone();
        let mem_limit_mb = ctx.server_config.limits.lua_memory_mb;
        let gc_full_count = gc_full_count.clone();
        let gc_full_micros = gc_full_micros.clone();
        let gc_freed_bytes = gc_freed_bytes.clone();
        let fn_ = lua.create_function(move |lua, ()| {
            use std::sync::atomic::Ordering;
            let uptime_secs = started_at.elapsed().as_secs_f64();
            let t = lua.create_table()?;
            t.set("version",     env!("CARGO_PKG_VERSION"))?;
            t.set("name",        cfg.game.name.clone())?;
            t.set("started_at",  started_at_utc.clone())?;
            t.set("uptime_secs", uptime_secs)?;
            // Output lost to full session channels. A non-zero value here is
            // the answer to "the MUD ate my text", which used to leave no
            // trace at all.
            t.set("dropped_output",
                sh.read_recover().dropped_output_total() as i64)?;

            // The Lua heap, in its own sub-table. `used_memory` is what mlua's
            // allocator has handed out, which is the same number
            // `collectgarbage("count")` reports in kilobytes — read here so a
            // caller does not have to remember the unit.
            let heap = lua.create_table()?;
            // Byte totals and counts are integers; only the ratio and the
            // duration are genuinely fractional. On LuaJIT every number was a
            // double and the distinction did not show, but from 5.3 on a count
            // returned as a float renders as `1.0` — which reached `mudstatus`
            // and every test that reads these.
            let used = lua.used_memory();
            heap.set("heap_bytes", used as i64)?;
            heap.set("heap_kb", used as f64 / 1024.0)?;
            heap.set("limit_bytes", (mem_limit_mb * 1024 * 1024) as i64)?;
            if mem_limit_mb > 0 {
                heap.set("heap_fraction",
                    used as f64 / (mem_limit_mb * 1024 * 1024) as f64)?;
            }
            heap.set("gc_full_count", gc_full_count.load(Ordering::Relaxed) as i64)?;
            heap.set("gc_full_ms",
                gc_full_micros.load(Ordering::Relaxed) as f64 / 1000.0)?;
            heap.set("gc_freed_bytes", gc_freed_bytes.load(Ordering::Relaxed) as i64)?;
            t.set("lua", heap)?;

            // Absent rather than zeroed when compute is off, so a mudlib can
            // tell "not running" from "running and idle".
            if let Some(bridge) = &compute {
                t.set("compute", super::efuns_compute::snapshot_table(lua, bridge)?)?;
            }
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
            // Send under the read lock rather than collecting senders first:
            // `Session::try_send` is what counts drops and emits the player's
            // truncation marker, and a bare `output_tx` clone skips both.
            // Sending is a `try_send` either way, so the lock is not held
            // across anything that can block.
            let handler = sh.read_recover();
            let mut count = 0usize;
            let mut dropped = 0usize;
            for sid in handler.all_ids() {
                if !handler.has_permission(&sid, &perm) {
                    continue;
                }
                if let Some(session) = handler.get(&sid) {
                    count += 1;
                    if !session.try_send(SessionOutput::Text(msg.clone())) {
                        dropped += 1;
                    }
                }
            }
            drop(handler);
            if dropped > 0 {
                tracing::warn!(
                    "broadcast_to_perm('{}') did not reach {} of {} session(s)",
                    perm, dropped, count
                );
            }
            Ok(count)
        })?;
        globals.set("broadcast_to_perm", fn_)?;
    }

    // verify_file(path) -> (bool, string?)
    // Compiles a mudlib or game file WITHOUT executing it.
    //
    // Through the same jail every other file efun uses. It had its own —
    // `sandbox::resolve_jailed_path`, which refuses any path containing `..`
    // and knew only the mudlib root — so `verify` disagreed with `read_file`
    // about which paths existed, and could not see the game layer at all. A
    // builder who can `cat` a file should be able to compile-check it.
    {
        let mudlib_path = ctx.mudlib_path.clone();
        let game_path = ctx
            .server_config
            .game
            .game_path
            .as_deref()
            .map(std::path::PathBuf::from);
        let perm_config = ctx.permission_config.clone();
        let sh = ctx.session_handler.clone();
        let gl = ctx.game_logger.clone();
        let fn_ = lua.create_function(move |lua, path: String| {
            check_efun_permission("verify_file", &perm_config, &sh, &gl)?;
            let (resolved, virt) = match crate::core::scripting::efuns_io::resolve_read_path(
                &mudlib_path,
                game_path.as_deref(),
                &path,
            ) {
                Ok(p) => p,
                Err(e) => return Ok((false, Some(format!("Path error: {}", e)))),
            };
            let code = match std::fs::read_to_string(&resolved) {
                Ok(c) => c,
                Err(e) => return Ok((false, Some(format!("Cannot read '{}': {}", virt, e)))),
            };
            // The chunk name is the *virtual* path, so a compile error, a
            // breakpoint and a stack frame all name the same file the builder
            // typed — including which layer it came from.
            let chunk_name = format!("@{}", virt);
            match lua.load(code.as_str()).set_name(&chunk_name).into_function() {
                Ok(_)  => Ok((true, None)),
                Err(e) => Ok((false, Some(e.to_string()))),
            }
        })?;
        globals.set("verify_file", fn_)?;
    }

    Ok(())
}


/// How deep a Lua table may nest before conversion refuses it.
///
/// Doubles as cycle detection: a self-referential table has no bottom, so it
/// trips this instead of recursing until the Rust stack is gone. It used to do
/// exactly that — `local t = {} t.t = t` handed to `save_character_data` took
/// the process down with no Lua error and nothing in the log.
const MAX_JSON_DEPTH: usize = 64;

/// How many values one conversion may visit. Bounds a table that is shallow
/// but enormous, a shared subtree copied once per reference, and a list so
/// sparse that filling its holes with nulls would allocate wildly.
const MAX_JSON_NODES: usize = 100_000;

/// What a Lua table can faithfully become in JSON.
///
/// JSON has one composite type; Lua has one that is a sequence and a map at
/// the same time. A table that is genuinely both has no faithful JSON form, so
/// it is refused rather than silently half-converted — which is what used to
/// happen, dropping every string key of `{"sword", "shield", gold = 100}` on
/// its way into the character save.
enum TableShape {
    /// No entries at all. Rendered as `{}` — neither JSON nor Lua can tell an
    /// empty list from an empty map.
    Empty,
    /// A list of this length. Holes become `null`, which round-trips back to a
    /// hole because `json_to_lua` skips nulls.
    Array(usize),
    /// Has keys, none of which form an array part.
    Object,
}

/// One step of the breadcrumb carried into conversion errors, so a failure
/// says *which* field is at fault rather than just that one is.
enum Step {
    Key(String),
    Index(usize),
}

fn render_path(path: &[Step]) -> String {
    if path.is_empty() {
        return "the value".to_string();
    }
    let mut rendered = String::new();
    for step in path {
        match step {
            Step::Key(k) if rendered.is_empty() => rendered.push_str(k),
            Step::Key(k) => {
                rendered.push('.');
                rendered.push_str(k);
            }
            Step::Index(i) => rendered.push_str(&format!("[{i}]")),
        }
    }
    format!("field `{rendered}`")
}

/// Decide how a table should be rendered, or explain why it cannot be.
fn classify_table(t: &LuaTable, path: &[Step]) -> LuaResult<TableShape> {
    let mut indices = 0usize;
    let mut max_index = 0i64;
    let mut named: Vec<String> = Vec::new();

    for pair in t.clone().pairs::<LuaValue, LuaValue>() {
        let (key, _) = pair?;
        match &key {
            // On LuaJIT every integral number arrives as `Integer`, so this is
            // the array-index case as well as the integer-key one.
            LuaValue::Integer(i) if *i >= 1 => {
                indices += 1;
                max_index = max_index.max(*i);
            }
            LuaValue::Integer(i) => named.push(i.to_string()),
            LuaValue::String(s) => named.push(s.to_string_lossy()),
            other => {
                return Err(LuaError::RuntimeError(format!(
                    "cannot convert {} to JSON: it has a key of type '{}', and JSON keys \
                     can only be strings or integers",
                    render_path(path),
                    other.type_name()
                )))
            }
        }
    }

    match (indices, named.is_empty()) {
        (0, true) => Ok(TableShape::Empty),
        (0, false) => Ok(TableShape::Object),
        (_, true) => Ok(TableShape::Array(max_index as usize)),
        (_, false) => {
            named.sort();
            let shown = named
                .iter()
                .take(4)
                .map(|k| format!("'{k}'"))
                .collect::<Vec<_>>()
                .join(", ");
            let more = if named.len() > 4 {
                format!(" and {} more", named.len() - 4)
            } else {
                String::new()
            };
            Err(LuaError::RuntimeError(format!(
                "cannot convert {} to JSON: it has {} list entr{} and also the key(s) \
                 {}{} — JSON has no type that is both a list and a map, so use one or \
                 the other",
                render_path(path),
                indices,
                if indices == 1 { "y" } else { "ies" },
                shown,
                more
            )))
        }
    }
}

/// Convert a Lua value to `serde_json::Value`.
///
/// Refuses anything JSON cannot hold rather than converting it approximately.
/// The previous version silently dropped the string keys of a mixed table,
/// turned NaN and infinity into `0`, turned functions into `null`, discarded
/// keys that were neither strings nor integers, and died on the Rust stack
/// when given a cycle — all of it on the `save_character_data` path, where
/// silence means a player's data is quietly wrong.
///
/// Callers are `pcall`-wrapped (`character_d.save`, `gmcp_d`), so raising here
/// surfaces in the journal instead of corrupting a save.
pub fn lua_to_json(lua: &Lua, val: &LuaValue) -> LuaResult<JsonValue> {
    let mut budget = MAX_JSON_NODES;
    let mut path = Vec::new();
    lua_to_json_inner(lua, val, 0, &mut budget, &mut path)
}

fn lua_to_json_inner(
    lua: &Lua,
    val: &LuaValue,
    depth: usize,
    budget: &mut usize,
    path: &mut Vec<Step>,
) -> LuaResult<JsonValue> {
    if *budget == 0 {
        return Err(LuaError::RuntimeError(format!(
            "cannot convert to JSON: more than {MAX_JSON_NODES} values, giving up at {}",
            render_path(path)
        )));
    }
    *budget -= 1;

    match val {
        LuaValue::Nil => Ok(JsonValue::Null),
        LuaValue::Boolean(b) => Ok(JsonValue::Bool(*b)),
        LuaValue::Integer(i) => Ok(JsonValue::Number((*i).into())),
        LuaValue::Number(n) => serde_json::Number::from_f64(*n)
            .map(JsonValue::Number)
            .ok_or_else(|| {
                LuaError::RuntimeError(format!(
                    "cannot convert {} to JSON: {n} has no JSON representation \
                     (NaN and infinity do not)",
                    render_path(path)
                ))
            }),
        LuaValue::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
        LuaValue::Table(t) => {
            if depth >= MAX_JSON_DEPTH {
                return Err(LuaError::RuntimeError(format!(
                    "cannot convert to JSON: nesting is deeper than {MAX_JSON_DEPTH} at {} \
                     — a table that refers to itself will always hit this",
                    render_path(path)
                )));
            }

            match classify_table(t, path)? {
                TableShape::Empty => Ok(JsonValue::Object(serde_json::Map::new())),
                TableShape::Array(len) => {
                    let mut arr = Vec::with_capacity(len.min(*budget));
                    for i in 1..=len {
                        path.push(Step::Index(i));
                        let v: LuaValue = t.get(i)?;
                        let converted = lua_to_json_inner(lua, &v, depth + 1, budget, path);
                        path.pop();
                        arr.push(converted?);
                    }
                    Ok(JsonValue::Array(arr))
                }
                TableShape::Object => {
                    let mut map = serde_json::Map::new();
                    for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                        let (k, v) = pair?;
                        // classify_table already rejected every other key type.
                        let key = match &k {
                            LuaValue::String(s) => s.to_str()?.to_string(),
                            LuaValue::Integer(i) => i.to_string(),
                            _ => continue,
                        };
                        path.push(Step::Key(key.clone()));
                        let converted = lua_to_json_inner(lua, &v, depth + 1, budget, path);
                        path.pop();
                        map.insert(key, converted?);
                    }
                    Ok(JsonValue::Object(map))
                }
            }
        }
        other => Err(LuaError::RuntimeError(format!(
            "cannot convert {} to JSON: a value of type '{}' has no JSON representation",
            render_path(path),
            other.type_name()
        ))),
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

