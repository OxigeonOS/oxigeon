use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use tokio::sync::mpsc;
use mlua::prelude::*;
use crate::core::lock::RwLockExt;

use crate::error::Result;
use super::efuns::{
    current_actor, enter_system_dispatch as set_system_identity, set_current_session, EfunContext,
};
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
    /// An off-thread compute job finished. See [`crate::core::compute`].
    ///
    /// Exactly one of these is delivered for every id `compute` handed out,
    /// whatever the outcome — success, failure, timeout, cancel, or a queue
    /// that was full — so the mudlib has one place where a job ends.
    ComputeResult {
        id: u64,
        /// `"ok"`, `"error"`, `"load_error"`, `"timeout"`, `"cancelled"`,
        /// `"budget"` or `"refused"`.
        kind: &'static str,
        value: crate::core::compute::LuaData,
        error: Option<String>,
        /// Whatever the caller attached at submit time, echoed back.
        tag: crate::core::compute::LuaData,
        module: String,
        func: String,
        queued_ms: f64,
        run_ms: f64,
        logs: Vec<(String, String)>,
    },
    /// An off-thread password hash finished. See [`crate::core::auth`].
    AuthResult {
        session_id: String,
        /// `"authenticate"` or `"create_account"`.
        kind: &'static str,
        /// The account table on success.
        account: Option<serde_json::Value>,
        /// A player-facing message on failure. Exactly one of these is `Some`.
        error: Option<String>,
    },
    /// Wake the Lua thread so it re-evaluates the debug hook.
    ///
    /// The thread otherwise parks in `blocking_recv` and would not notice a
    /// debug client attaching or detaching until some other event arrived.
    DebugSync,
    /// Dispatch `on_shutdown` and then shut down the Lua VM.
    Shutdown,
}

/// The Lua scripting engine — runs on a dedicated thread.
pub struct ScriptEngine {
    /// Send commands to the Lua thread
    pub cmd_tx: mpsc::UnboundedSender<LuaCommand>,
    /// Handle to the Lua thread
    thread_handle: Option<JoinHandle<()>>,
    /// Signalled by the Lua thread once its event loop has ended — that is,
    /// after `on_shutdown` has returned.
    ///
    /// A `JoinHandle` cannot be joined with a deadline, and a mudlib that
    /// wedges in `on_shutdown` must not hang the process forever. Waiting on
    /// this instead makes the wait bounded. See [`ScriptEngine::shutdown_within`].
    finished: std::sync::mpsc::Receiver<()>,
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

        // Sent once the event loop below has ended, so a caller can wait for
        // the mudlib's `on_shutdown` to finish without joining unconditionally.
        let (finished_tx, finished) = std::sync::mpsc::channel::<()>();

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

            // Memory ceiling, before anything can allocate against it.
            //
            // mlua returns `MemoryControlNotAvailable` on builds where the
            // allocator is not under its control, which is why this reports
            // rather than assumes. This one enforces it: an allocation past the
            // ceiling raises a catchable Lua error and the VM stays usable
            // afterwards, so one greedy command cannot take the game down.
            let mem_limit_mb = ctx.server_config.limits.lua_memory_mb;
            if mem_limit_mb > 0 {
                match lua.set_memory_limit(mem_limit_mb * 1024 * 1024) {
                    Ok(_) => tracing::info!("Lua memory limit: {} MB", mem_limit_mb),
                    Err(e) => tracing::warn!(
                        "limits.lua_memory_mb is not enforceable on this build: {}",
                        e
                    ),
                }
            }

            // Register all efuns
            if let Err(e) = super::efuns::register_all(&lua, &ctx) {
                tracing::error!("Failed to register efuns: {}", e);
                return;
            }

            // An instruction budget only works in the interpreter, so arming
            // one means giving up the JIT. See `disable_jit_for_budget`.
            //
            // `OXIGEON_JIT=off` turns the compiler off *without* arming a
            // budget. That combination exists only for `benches/dispatch.rs`:
            // the config key alone cannot separate "lost the JIT" from "gained
            // a hook", because setting it does both, and a benchmark that
            // moves two variables at once measures neither. It is read once,
            // at startup, and there is deliberately no Lua-side equivalent —
            // `apply_sandbox` removes the `jit` table entirely.
            let jit_off_requested = std::env::var("OXIGEON_JIT").as_deref() == Ok("off");
            if (debug_state.instruction_limit > 0 || jit_off_requested)
                && !disable_jit_for_budget(&lua, jit_off_requested)
            {
                tracing::error!(
                    "could not disable the LuaJIT compiler — limits.lua_instruction_limit \
                     cannot be enforced and a runaway loop would wedge the game thread"
                );
                return;
            }

            // Close the sandbox. This runs after `register_all` (so the jailed
            // efuns are already in place as the replacement for what it strips)
            // and before any mudlib code loads. Refusing to serve is the right
            // outcome if it fails — the alternative is a game that silently
            // hands `io.popen` to anyone who can write a room file.
            if let Err(e) = super::sandbox::apply_sandbox(&lua) {
                tracing::error!("Failed to apply the Lua sandbox: {}", e);
                return;
            }

            // Seed the PRNG before any mudlib code can roll anything. LuaJIT
            // starts from a constant, so without this every boot replayed the
            // same combat, the same loot and the same weighted echoes. Salt 0
            // is the game VM; compute workers use their index.
            if let Err(e) = super::sandbox::seed_prng(&lua, 0) {
                tracing::error!("Failed to seed the Lua PRNG: {}", e);
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

            // Loading the two layers is engine-internal: no player exists yet,
            // and an init file that calls a gated efun (registering a daemon's
            // storage, say) would otherwise be refused with nobody to see it.
            let startup_identity = set_system_identity();

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

            // Startup is over; from here every dispatch declares its own
            // identity, and anything that does not have one has none.
            drop(startup_identity);

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
                        // Every dispatch starts with a full instruction budget.
                        // Loading the mudlib above deliberately runs unbudgeted:
                        // it is trusted startup code, and the whole game easily
                        // costs more instructions than any one command may.
                        debugger::hook::begin_budget(&hook_local);
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
                                // A reload runs a whole module's top level. It
                                // is engine-internal by the time it gets here —
                                // whichever admin asked for it is long out of
                                // scope — so it acts as the engine.
                                let _system = set_system_identity();
                                hot_reload(&lua, &module_name, &mudlib_path, &game_path);
                            }
                            LuaCommand::TimerFired { id } => {
                                // No player is behind a tick. Without this the
                                // dispatch has no identity at all and every
                                // gated efun a daemon calls is denied — see
                                // `efuns::enter_system_dispatch`.
                                let _system = set_system_identity();
                                dispatch_event(&lua, "on_timer", &[id], &game_logger, &session_handler_for_log);
                            }
                            LuaCommand::ComputeResult {
                                id, kind, value, error, tag, module, func,
                                queued_ms, run_ms, logs,
                            } => {
                                // No player is behind a compute result, the
                                // same as a timer tick — so it dispatches with
                                // the engine's own identity rather than none.
                                let _system = set_system_identity();
                                dispatch_compute_result(
                                    &lua, id, kind, &value, error.as_deref(), &tag,
                                    &module, &func, queued_ms, run_ms, &logs,
                                    &game_logger, &session_handler_for_log,
                                );
                            }
                            LuaCommand::AuthResult { session_id, kind, account, error } => {
                                // The session is set for the same reason it is
                                // on OnInput: everything the login hook goes on
                                // to call — `authenticate_session`, the world
                                // and character daemons — reads it.
                                set_current_session(Some(session_id.clone()));
                                dispatch_auth_result(
                                    &lua, &session_id, kind, account.as_ref(), error.as_deref(),
                                    &game_logger, &session_handler_for_log,
                                );
                                set_current_session(None);
                            }
                            // The sync_hook call above is the entire point.
                            LuaCommand::DebugSync => {}
                            LuaCommand::Shutdown => {
                                // The mudlib's last chance to flush what it is
                                // holding in memory. `CHARACTER_D` is a
                                // write-back cache emptied by the autosave
                                // ticker, so without this dispatch a clean
                                // restart silently discards everything since
                                // the last tick.
                                //
                                // No player is behind a shutdown, so it runs
                                // under the engine's own identity for the same
                                // reason a timer tick does — otherwise every
                                // gated efun the flush touches is denied.
                                let _system = set_system_identity();
                                dispatch_event(&lua, "on_shutdown", &[], &game_logger, &session_handler_for_log);
                                tracing::info!("Lua engine shutting down");
                                break;
                            }
                        }
                    }
                }
            }

            // Whoever asked for the shutdown is waiting on this. Dropping the
            // VM comes after: the saving is done by here, and a caller that
            // gave up waiting must not be blocked by LuaJIT teardown.
            let _ = finished_tx.send(());
        });

        Ok(ScriptEngine {
            cmd_tx,
            thread_handle: Some(thread_handle),
            finished,
        })
    }

    /// Send a command to the Lua thread. Non-blocking.
    pub fn send(&self, cmd: LuaCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Ask the mudlib to shut down and wait — up to `timeout` — for it.
    ///
    /// The Lua thread dispatches `on_shutdown` before it stops, which is where
    /// the mudlib flushes anything it holds in memory. Returning without
    /// waiting would race that flush against process exit, so this blocks; but
    /// a mudlib that wedges in `on_shutdown` must not wedge the server's
    /// shutdown with it, so the wait is bounded.
    ///
    /// Returns `false` if the timeout expired first — the caller should log
    /// that and exit anyway, accepting whatever the flush did not finish.
    pub fn shutdown_within(&self, timeout: std::time::Duration) -> bool {
        if self.cmd_tx.send(LuaCommand::Shutdown).is_err() {
            // The thread is already gone; nothing to wait for.
            return true;
        }
        match self.finished.recv_timeout(timeout) {
            Ok(()) => true,
            // Disconnected means the thread ended without signalling — it
            // panicked. Not a clean shutdown, but not something waiting fixes.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => true,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
        }
    }

    /// Shut down the Lua VM and wait for the thread to finish, however long it
    /// takes. Prefer [`ScriptEngine::shutdown_within`] anywhere a wedged mudlib
    /// could hang a user-visible path.
    pub fn shutdown(mut self) {
        let _ = self.cmd_tx.send(LuaCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Turn off the LuaJIT compiler so the instruction budget can be enforced.
///
/// The hook is the only way to interrupt Lua, and LuaJIT does not dispatch
/// hooks from inside a compiled trace. Measured on this build, a one-line
/// `while true do s = s + 1 end` delivers **no** hook events at all with the
/// JIT on — not count, not line, not call. It is not that the count trigger is
/// too coarse; the events never arrive. So an enforced limit and the compiler
/// are mutually exclusive, and the config key chooses between them.
///
/// The price, measured through the real mudlib by `benches/dispatch.rs`:
///
/// ```text
///                     look    who   mudstatus   numeric loop (control)
///   cost of enforcing 1.03x  1.02x    1.07x            2.59x
///   compiler worth    1.00x  1.01x    1.06x            2.10x
/// ```
///
/// The compiler earns its keep on arithmetic and almost nothing on command
/// dispatch: nothing in one command loops the 56 times LuaJIT needs to start a
/// trace, and the dispatcher's `gsub`/`match`/`gmatch` calls abort tracing
/// anyway. Hence the limit being on by default.
///
/// `jit.off()` only stops *new* traces, so this must run before any mudlib
/// code — which it does. `apply_sandbox` then removes the `jit` table outright,
/// otherwise a single `jit.on()` in a room file would hand the compiler back
/// and silently disarm the budget.
fn disable_jit_for_budget(lua: &Lua, by_env: bool) -> bool {
    if let Err(e) = lua.load("jit.off()").set_name("=<oxigeon>/jit_off").exec() {
        tracing::error!("jit.off() failed: {}", e);
        return false;
    }
    if by_env {
        tracing::warn!(
            "LuaJIT compiler disabled by OXIGEON_JIT=off — this is a benchmarking \
             control, not a supported way to run a server"
        );
    } else {
        tracing::info!(
            "LuaJIT compiler disabled so limits.lua_instruction_limit can be enforced \
             (set it to 0 to keep the compiler and drop the limit)"
        );
    }
    true
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

    // `current_actor` rather than the raw session: an error raised on a timer
    // tick is attributed to "system", not to "unknown".
    let sid = current_actor();
    let char_id = {
        if let Ok(id) = sid.parse::<crate::core::session::SessionId>() {
            sh.read_recover().get(&id).and_then(|s| s.state.character_id())
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

/// Deliver a finished compute job to the mudlib.
///
/// Calls `on_compute_result(id, ok, value, err, meta)`. A mudlib with no such
/// global gets a loud error rather than silence, for the same reason
/// `on_auth_result` does: the caller would otherwise wait forever for an
/// answer that was produced and thrown away.
#[allow(clippy::too_many_arguments)]
fn dispatch_compute_result(
    lua: &Lua,
    id: u64,
    kind: &str,
    value: &crate::core::compute::LuaData,
    error: Option<&str>,
    tag: &crate::core::compute::LuaData,
    module: &str,
    func: &str,
    queued_ms: f64,
    run_ms: f64,
    logs: &[(String, String)],
    gl: &Arc<crate::core::logging::GameLogger>,
    sh: &Arc<std::sync::RwLock<crate::core::session::SessionHandler>>,
) {
    use crate::core::compute::marshal;

    let Ok(func_ref) = lua.globals().get::<LuaFunction>("on_compute_result") else {
        tracing::error!(
            "mudlib defines no on_compute_result — the {} result for job {} ({}.{}) is \
             lost, and whatever asked for it will wait forever",
            kind, id, module, func
        );
        return;
    };

    // A job's own log lines go to the journal from here, on the Lua thread,
    // rather than from the worker — the worker is not rate-limited and nothing
    // there knows the job's identity.
    for (level, message) in logs {
        gl.journal(crate::core::logging::JournalEntry {
            level,
            source: "compute",
            message,
            meta: Some(serde_json::json!({"job": id, "module": module, "fn": func})),
        });
    }

    let build_args = || -> LuaResult<(u64, bool, LuaValue, Option<String>, LuaTable)> {
        let meta = lua.create_table()?;
        meta.set("kind", kind)?;
        meta.set("module", module)?;
        meta.set("fn", func)?;
        meta.set("queued_ms", queued_ms)?;
        meta.set("run_ms", run_ms)?;
        meta.set("tag", marshal::to_lua(lua, tag)?)?;
        Ok((
            id,
            kind == "ok",
            marshal::to_lua(lua, value)?,
            error.map(str::to_string),
            meta,
        ))
    };

    let args = match build_args() {
        Ok(args) => args,
        Err(e) => {
            tracing::error!("compute: could not build the result for job {}: {}", id, e);
            return;
        }
    };

    if let Err(e) = func_ref.call::<()>(args) {
        tracing::error!("Lua error in on_compute_result: {}", e);
        log_lua_error(gl, sh, "on_compute_result", &e,
            Some(serde_json::json!({"job": id, "module": module, "fn": func})));
    }
}

/// Deliver an off-thread authentication result to the mudlib.
///
/// Calls `on_auth_result(session_id, kind, account_or_nil, error_or_nil)`.
/// A mudlib with no such global gets a loud error rather than silence: the
/// player would otherwise be left staring at a password prompt that never
/// answers, which is indistinguishable from the server having hung.
fn dispatch_auth_result(
    lua: &Lua,
    session_id: &str,
    kind: &str,
    account: Option<&serde_json::Value>,
    error: Option<&str>,
    gl: &Arc<crate::core::logging::GameLogger>,
    sh: &Arc<std::sync::RwLock<crate::core::session::SessionHandler>>,
) {
    let Ok(func) = lua.globals().get::<LuaFunction>("on_auth_result") else {
        tracing::error!(
            "mudlib defines no on_auth_result — the {} result for session {} is lost \
             and that player's login will never complete",
            kind,
            session_id
        );
        return;
    };

    let account_val = match account {
        Some(json) => match super::efuns::json_to_lua(lua, json) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Could not convert the account table for Lua: {}", e);
                LuaValue::Nil
            }
        },
        None => LuaValue::Nil,
    };

    let args = (session_id.to_string(), kind.to_string(), account_val, error.map(str::to_string));
    if let Err(e) = func.call::<()>(args) {
        tracing::error!("Lua error in on_auth_result: {}", e);
        log_lua_error(gl, sh, "on_auth_result", &e, Some(serde_json::json!({"kind": kind})));
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
        // A backstop for teardown paths that never got to ask politely — a
        // failed startup, a test dropping its VM, a panic unwinding past the
        // driver. It only sends; it does not wait, so on this path the flush
        // races process exit. Anything that cares about the data going to disk
        // calls `shutdown_within` first.
        let _ = self.cmd_tx.send(LuaCommand::Shutdown);
    }
}
