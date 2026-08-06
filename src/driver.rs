use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use crate::core::lock::RwLockExt;

use crate::config::{DriverConfig, ServerConfig};
use crate::config::driver_config::DatabaseBackend;
use crate::config::PermissionConfig;
use crate::core::{
    TelnetListener,
    Session, SessionId, SessionOutput, SessionHandler,
    ScriptEngine, LuaCommand, EfunContext,
};
use crate::core::logging::{GameLogger, utc_now};
use crate::core::network::telnet::{TelnetParser, TelnetEvent, TelnetCodec};
use crate::core::network::telnet::constants::*;
use crate::domain::db::connection::AnyPool;
use crate::domain::models::{DieselAccountStore, DieselCharacterStore};
use crate::domain::models::role::DieselRoleStore;
use crate::error::{OxigeonError, Result};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// The central driver — owns all subsystems and coordinates them.
pub struct Driver {
    pub driver_config: DriverConfig,
    pub server_config: ServerConfig,
    pub session_handler: Arc<RwLock<SessionHandler>>,
    pub script_engine: ScriptEngine,
    pub db_pool: AnyPool,
    pub debug_state: crate::core::scripting::debugger::SharedDebugState,
    pub auth_worker: crate::core::auth::AuthWorker,
    pub compute: Option<crate::core::compute::ComputeBridge>,
}

impl Driver {
    pub async fn new(driver_config: DriverConfig, server_config: ServerConfig) -> Result<Self> {
        // 1. Establish database pool
        let db_pool = AnyPool::new(&driver_config.database)?;
        tracing::info!("Database pool established ({})",
            match driver_config.database.backend {
                DatabaseBackend::Sqlite => "SQLite",
                DatabaseBackend::Postgresql => "PostgreSQL",
            }
        );

        // 2. Run migrations
        {
            let mut conn = db_pool.get_sqlite()?;
            conn.run_pending_migrations(MIGRATIONS)
                .map_err(|e| OxigeonError::Internal(format!("Migration failed: {}", e)))?;
            tracing::info!("Database migrations applied");
        }

        // 3. Create stores
        let account_store = Arc::new(DieselAccountStore::new(
            db_pool.clone(),
            server_config.accounts.min_password_length,
        ));
        let character_store = Arc::new(DieselCharacterStore::new(
            db_pool.clone(),
            server_config.accounts.max_characters_per_account,
        ));
        let role_store = Arc::new(DieselRoleStore::new(db_pool.clone()));
        // One table serves every collection a game invents, so a game author
        // never needs a migration. See docs/src/lua-api/document-store.md.
        let document_store = Arc::new(crate::domain::models::DieselDocumentStore::new(
            db_pool.clone(),
            server_config.documents.clone(),
        )?);

        // 4. Create session handler
        let session_handler = Arc::new(RwLock::new(SessionHandler::new(
            server_config.sessions.multisession_mode.clone(),
            server_config.sessions.max_connections,
        )));

        // 5. Start Lua scripting engine
        // Pre-create the channel so cmd_tx can be given to EfunContext
        // (enabling the Lua-callable `reload()` efun)
        let mudlib_path = PathBuf::from(&server_config.game.mudlib_path);
        let config_dir = std::path::Path::new("config");
        let permission_config = PermissionConfig::load_from_file(&config_dir.join("permissions.toml"));
        // A directory rule that names no root protects nothing, and the file
        // efuns are jailed to two trees now. `load_from_file` already logged
        // each one; say it again at startup, where an operator is looking, so a
        // boundary somebody believes in cannot quietly not exist. That is
        // exactly how the `/areas` rule spent months commented out.
        if !permission_config.invalid_directory_keys.is_empty() {
            tracing::error!(
                "permissions.toml: {} directory rule(s) name no root and are NOT in \
                 effect: {}. Prefix each with /mudlib or /game.",
                permission_config.invalid_directory_keys.len(),
                permission_config.invalid_directory_keys.join(", ")
            );
        }
        let (engine_cmd_tx, engine_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<crate::core::scripting::engine::LuaCommand>();

        // 5a. Game logger
        let log_dir = std::path::Path::new("logs");
        let game_logger = Arc::new(GameLogger::new(log_dir));
        let started_at = std::time::Instant::now();
        let started_at_utc = utc_now();
        tracing::info!("Game logging to {:?}/{{audit,journal}}.log", log_dir);

        // Shared with the DAP listener started in `run()`. Created here because
        // the Lua thread claims its request channel at startup.
        let debug_cfg = driver_config.servers.debug.clone().unwrap_or_default();
        let debug_state = crate::core::scripting::debugger::DebugState::from_config(
            &debug_cfg,
            server_config.limits.lua_instruction_limit,
        );
        match server_config.limits.lua_instruction_limit {
            0 => tracing::info!(
                "limits.lua_instruction_limit is 0 — the LuaJIT compiler stays on, and a \
                 runaway loop in Lua will wedge the game thread until the process is killed"
            ),
            n => tracing::info!("Lua instruction limit: {} per dispatch", n),
        }

        // Two workers: enough that one slow verify does not stall the next
        // login, few enough that a flood of attempts cannot turn into a fan of
        // CPU-bound threads competing with the game thread.
        let auth_worker = crate::core::auth::AuthWorker::start(
            account_store.clone(),
            engine_cmd_tx.clone(),
            2,
        );

        // The compute pool. `None` unless [compute] enabled = true, which is
        // what keeps the efuns unregistered and the feature free when unused.
        let compute = crate::core::compute::ComputeBridge::start(
            server_config.compute.clone(),
            mudlib_path.clone(),
            PathBuf::from(server_config.game.game_path.as_deref().unwrap_or("./game")),
            engine_cmd_tx.clone(),
        );

        let efun_ctx = EfunContext {
            session_handler: session_handler.clone(),
            account_store: account_store.clone(),
            character_store: character_store.clone(),
            role_store,
            server_config: Arc::new(server_config.clone()),
            mudlib_path: mudlib_path.clone(),
            cmd_tx: Some(engine_cmd_tx.clone()),
            permission_config: Arc::new(permission_config),
            game_logger,
            started_at,
            started_at_utc,
            debug_state: debug_state.clone(),
            auth_worker: Some(auth_worker.clone()),
            compute: compute.clone(),
            document_store,
        };

        let script_engine = ScriptEngine::start(mudlib_path, efun_ctx, engine_cmd_tx, engine_cmd_rx)?;
        tracing::info!("Lua scripting engine started");

        Ok(Driver {
            driver_config,
            server_config,
            session_handler,
            script_engine,
            db_pool,
            debug_state,
            auth_worker,
            compute,
        })
    }

    /// Main server loop — accept connections and dispatch events
    pub async fn run(&self) -> Result<()> {
        let telnet_config = self.driver_config.servers.telnet.clone()
            .ok_or_else(|| OxigeonError::Config("No telnet server configured".into()))?;

        if !telnet_config.enabled {
            tracing::warn!("Telnet server is disabled in config");
            return Ok(());
        }

        // Debug adapter, if configured. Started before the accept loop so an
        // editor can attach while the game is still idle.
        if let Some(dbg) = self.driver_config.servers.debug.as_ref().filter(|d| d.enabled) {
            match crate::core::scripting::debugger::dap::serve(
                &dbg.bind,
                dbg.port,
                self.debug_state.clone(),
            )
            .await
            {
                Ok(addr) => {
                    tracing::info!("Lua debug adapter listening on {}", addr);
                    if !addr.ip().is_loopback() {
                        tracing::warn!(
                            "debug adapter bound to {} — it grants unauthenticated control \
                             of the game VM and must not be reachable off-host",
                            addr.ip()
                        );
                    }
                }
                Err(e) => tracing::error!("Failed to start debug adapter: {}", e),
            }
        }

        let mut listener = TelnetListener::new(telnet_config);
        listener.start().await?;

        tracing::info!("Oxigeon v{} started — accepting connections on {}",
            env!("CARGO_PKG_VERSION"), listener.addr());

        // Compute deadlines are watched by a thread the bridge owns, not from
        // here — see `ComputeBridge::spawn_watchdog`.
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((conn, reader, addr)) => {
                            let sh = self.session_handler.clone();
                            let cmd_tx = self.script_engine.cmd_tx.clone();
                            let max_buf = self.server_config.limits.input_buffer_bytes;
                            let auth = self.auth_worker.clone();
                            tokio::spawn(async move {
                                handle_connection(conn, reader, addr, sh, cmd_tx, max_buf, auth).await;
                            });
                        }
                        Err(e) => {
                            tracing::error!("Accept error: {}", e);
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Shutdown signal received");
                    {
                        let handler = self.session_handler.read_recover();
                        handler.broadcast("\r\nServer shutting down. Goodbye!\r\n");
                    }
                    break;
                }
            }
        }

        self.shutdown_lua();
        Ok(())
    }

    /// Ask the mudlib to flush, and wait for it.
    ///
    /// Ordering is the whole point. `Drop for ScriptEngine` sends the same
    /// command but never waits, so relying on it meant the process could exit
    /// before the Lua thread had even read the message — and nothing asked the
    /// mudlib to save in the first place. Since `CHARACTER_D` only reaches the
    /// database on an autosave tick, that discarded up to `autosave_seconds`
    /// of every online player's progress on every clean restart.
    fn shutdown_lua(&self) {
        let timeout = self.server_config.game.shutdown_timeout();
        tracing::info!("Flushing game state (waiting up to {:?})", timeout);
        if self.script_engine.shutdown_within(timeout) {
            tracing::info!("Game state flushed");
        } else {
            tracing::error!(
                "on_shutdown did not finish within {:?} — exiting anyway. Player data \
                 changed since the last autosave may be lost; look for a mudlib \
                 on_shutdown that blocks, or raise game.shutdown_timeout_seconds",
                timeout
            );
        }
    }
}

/// Handle a single client connection for its entire lifetime.
/// Implements the full bidirectional relay loop:
///   TCP read → Telnet parse → Lua on_input
///   Lua send → SessionOutput → TCP write
async fn handle_connection(
    mut conn: crate::core::network::telnet::TelnetConnection,
    mut reader: tokio::net::tcp::OwnedReadHalf,
    addr: SocketAddr,
    session_handler: Arc<RwLock<SessionHandler>>,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<LuaCommand>,
    max_buf: usize,
    auth_worker: crate::core::auth::AuthWorker,
) {
    // Create output channel for this session
    let (output_tx, mut output_rx) = mpsc::channel::<SessionOutput>(64);

    // Create session
    let session = Session::new("telnet".to_string(), addr, output_tx);
    let session_id = session.id;
    let session_id_str = session_id.to_string();

    // Register with handler — drop the guard before awaiting
    let connect_result = {
        let mut handler = session_handler.write_recover();
        handler.connect(session)
    };

    if let Err(e) = connect_result {
        tracing::warn!("Cannot register session from {}: {}", addr, e);
        let _ = conn.send_text("\r\nServer is full. Try again later.\r\n").await;
        return;
    }

    tracing::info!("Connection accepted: {} ({})", session_id_str, addr);

    // Send initial telnet negotiations
    if let Err(e) = conn.send_initial_negotiations().await {
        tracing::warn!("Negotiation error for {}: {}", session_id_str, e);
    }

    // Notify Lua on_connect
    let _ = cmd_tx.send(LuaCommand::OnConnect {
        session_id: session_id_str.clone(),
    });

    // ── Full bidirectional relay loop ──────────────────────────
    // Reads from TCP, parses Telnet, dispatches on_input.
    // Reads from output_rx, encodes, writes to TCP.
    let mut parser = TelnetParser::new();
    let mut buf = vec![0u8; max_buf.max(4096)];
    let mut line_buf = String::new();

    loop {
        tokio::select! {
            // ── Read from TCP ──────────────────────
            result = reader.read(&mut buf) => {
                match result {
                    Ok(0) => {
                        // Client closed connection
                        break;
                    }
                    Ok(n) => {
                        // Feed bytes to Telnet parser
                        for &byte in &buf[..n] {
                            if let Some(event) = parser.feed(byte) {
                                match event {
                                    TelnetEvent::Data(bytes) => {
                                        let text = TelnetCodec::decode_line(&bytes);
                                        line_buf.push_str(&text);
                                    }
                                    TelnetEvent::Negotiate { verb, option } => {
                                        handle_negotiation(&mut conn, &session_id_str, verb, option, &cmd_tx).await;
                                        publish_capabilities(&session_handler, session_id, &conn.capabilities);
                                    }
                                    TelnetEvent::Subnegotiation { option, data } => {
                                        handle_subnegotiation(&mut conn, &session_id_str, option, &data, &cmd_tx).await;
                                        publish_capabilities(&session_handler, session_id, &conn.capabilities);
                                    }
                                    TelnetEvent::Command(_) => {}
                                }
                            }
                        }
                        // Flush remaining data
                        if let Some(event) = parser.flush() {
                            if let TelnetEvent::Data(bytes) = event {
                                let text = TelnetCodec::decode_line(&bytes);
                                line_buf.push_str(&text);
                            }
                        }

                        // Check for complete lines (terminated by \n)
                        while let Some(nl_pos) = line_buf.find('\n') {
                            let line: String = line_buf.drain(..=nl_pos).collect();
                            let line = line.trim_end_matches(|c| c == '\r' || c == '\n').to_string();

                            let _ = cmd_tx.send(LuaCommand::OnInput {
                                session_id: session_id_str.clone(),
                                text: line,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Read error for {}: {}", session_id_str, e);
                        break;
                    }
                }
            }

            // ── Process output from Lua ──────────────
            msg = output_rx.recv() => {
                match msg {
                    Some(SessionOutput::Text(text)) => {
                        let _ = conn.send_text(&text).await;
                    }
                    Some(SessionOutput::Raw(bytes)) => {
                        let _ = conn.send_raw(&bytes).await;
                    }
                    Some(SessionOutput::Gmcp { package, data }) => {
                        let json = data.to_string();
                        let _ = conn.send_gmcp(&package, Some(&json)).await;
                    }
                    Some(SessionOutput::StartEcho) => {
                        let _ = conn.start_echo().await;
                    }
                    Some(SessionOutput::StopEcho) => {
                        let _ = conn.stop_echo().await;
                    }
                    Some(SessionOutput::Disconnect) | None => {
                        break;
                    }
                }
            }
        }
    }

    // ── Cleanup ───────────────────────────────────────────
    {
        let mut handler = session_handler.write_recover();
        handler.disconnect(&session_id);
    }

    let _ = cmd_tx.send(LuaCommand::OnDisconnect {
        session_id: session_id_str.clone(),
    });

    // Drop this address's failed-login tally, unless it is actually locked
    // out — otherwise reconnecting would be a free reset.
    auth_worker.forget(Some(addr.ip()));

    let _ = conn.close().await;
    tracing::info!("Connection closed: {} ({})", session_id_str, addr);
}

/// Copy what negotiation discovered onto the **Session**.
///
/// Negotiation writes to `TelnetConnection.capabilities`; the mudlib reads
/// `Session.capabilities`, through `get_session`. They are two structs on two
/// objects and nothing joined them, so `Session.capabilities` sat at
/// `Default::default()` for the life of every session that has ever connected.
///
/// The consequences were all silent. `gmcp_d` guards every one of its four
/// senders on `sess.gmcp_supported`, so **no GMCP was ever pushed to any
/// client** — the TUI's Room.Info, Char.Vitals and Effects panes could not
/// populate, and the `Core.Hello` a client does receive comes straight from
/// `handle_negotiation` and never touches Lua, which is what made the link look
/// healthy. `window_width` was nil too, so output was wrapped to a default
/// regardless of the terminal's real size.
///
/// Called after every negotiation and subnegotiation rather than once at the
/// end: NAWS arrives again on every resize, and TTYPE can arrive well after the
/// first GMCP message.
fn publish_capabilities(
    session_handler: &Arc<RwLock<SessionHandler>>,
    session_id: SessionId,
    caps: &crate::core::network::telnet::ClientCapabilities,
) {
    let mut handler = session_handler.write_recover();
    if let Some(session) = handler.get_mut(&session_id) {
        session.capabilities = caps.clone();
    }
}

/// Handle a Telnet negotiation event.
async fn handle_negotiation(
    conn: &mut crate::core::network::telnet::TelnetConnection,
    _session_id: &str,
    verb: u8,
    option: u8,
    _cmd_tx: &tokio::sync::mpsc::UnboundedSender<LuaCommand>,
) {
    let response = match verb {
        WILL => {
            let (cmd, _) = conn.negotiator.receive_will(option);
            cmd
        }
        WONT => {
            let (cmd, _) = conn.negotiator.receive_wont(option);
            cmd
        }
        DO => {
            let (cmd, _) = conn.negotiator.receive_do(option);
            cmd
        }
        DONT => {
            let (cmd, _) = conn.negotiator.receive_dont(option);
            cmd
        }
        _ => None,
    };

    if let Some(cmd) = response {
        let _ = conn.send_raw(&cmd.to_bytes()).await;
    }

    // Note GMCP negotiation
    if option == OPT_GMCP && (verb == DO || verb == WILL) {
        conn.capabilities.gmcp_supported = true;
        // Send server identification via GMCP
        let _ = conn.send_gmcp("Core.Hello", Some(r#"{"client":"Oxigeon","version":"0.1.0"}"#)).await;
    }

    // Note MCCP2 negotiation
    if option == OPT_MCCP2 && verb == DO {
        conn.capabilities.mccp2_supported = true;
    }
}

/// Handle a Telnet subnegotiation event.
async fn handle_subnegotiation(
    conn: &mut crate::core::network::telnet::TelnetConnection,
    session_id: &str,
    option: u8,
    data: &[u8],
    cmd_tx: &tokio::sync::mpsc::UnboundedSender<LuaCommand>,
) {
    match option {
        OPT_TTYPE => {
            // Terminal type report: SEND = 0x01 comes first, then IS = 0x00 + type string
            if data.first() == Some(&0x00) {
                // IS prefix
                let ttype = String::from_utf8_lossy(&data[1..]).to_string();
                conn.capabilities.terminal_type = Some(ttype);
                tracing::debug!("Terminal type for {}: {:?}", session_id, conn.capabilities.terminal_type);
            }
        }
        OPT_NAWS => {
            // Window size: 4 bytes — width (2 bytes big-endian) + height (2 bytes big-endian)
            if data.len() >= 4 {
                let width = u16::from_be_bytes([data[0], data[1]]);
                let height = u16::from_be_bytes([data[2], data[3]]);
                conn.capabilities.window_width = Some(width);
                conn.capabilities.window_height = Some(height);
                tracing::debug!("NAWS for {}: {}x{}", session_id, width, height);
            }
        }
        OPT_GMCP => {
            // GMCP message: "Package.Name" [space json]
            if let Some((package, json_opt)) = TelnetCodec::parse_gmcp(data) {
                let json_val: serde_json::Value = json_opt
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or(serde_json::Value::Null);

                let _ = cmd_tx.send(LuaCommand::OnGmcp {
                    session_id: session_id.to_string(),
                    package,
                    data: json_val,
                });
            }
        }
        _ => {}
    }
}
