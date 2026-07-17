use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use crate::config::{DriverConfig, ServerConfig};
use crate::config::driver_config::DatabaseBackend;
use crate::config::PermissionConfig;
use crate::core::{
    TelnetListener,
    Session, SessionOutput, SessionHandler,
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
        let (engine_cmd_tx, engine_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<crate::core::scripting::engine::LuaCommand>();

        // 5a. Game logger
        let log_dir = std::path::Path::new("logs");
        let game_logger = Arc::new(GameLogger::new(log_dir));
        let started_at = std::time::Instant::now();
        let started_at_utc = utc_now();
        tracing::info!("Game logging to {:?}/{{audit,journal}}.log", log_dir);

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
        };

        let script_engine = ScriptEngine::start(mudlib_path, efun_ctx, engine_cmd_tx, engine_cmd_rx)?;
        tracing::info!("Lua scripting engine started");

        Ok(Driver {
            driver_config,
            server_config,
            session_handler,
            script_engine,
            db_pool,
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

        let mut listener = TelnetListener::new(telnet_config);
        listener.start().await?;

        tracing::info!("Oxigeon v{} started — accepting connections on {}",
            env!("CARGO_PKG_VERSION"), listener.addr());

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((conn, reader, addr)) => {
                            let sh = self.session_handler.clone();
                            let cmd_tx = self.script_engine.cmd_tx.clone();
                            let max_buf = self.server_config.limits.input_buffer_bytes;
                            tokio::spawn(async move {
                                handle_connection(conn, reader, addr, sh, cmd_tx, max_buf).await;
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
                        let handler = self.session_handler.read().unwrap();
                        handler.broadcast("\r\nServer shutting down. Goodbye!\r\n");
                    }
                    break;
                }
            }
        }

        Ok(())
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
) {
    // Create output channel for this session
    let (output_tx, mut output_rx) = mpsc::channel::<SessionOutput>(64);

    // Create session
    let session = Session::new("telnet".to_string(), addr, output_tx);
    let session_id = session.id;
    let session_id_str = session_id.to_string();

    // Register with handler — drop the guard before awaiting
    let connect_result = {
        let mut handler = session_handler.write().unwrap();
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
                                    }
                                    TelnetEvent::Subnegotiation { option, data } => {
                                        handle_subnegotiation(&mut conn, &session_id_str, option, &data, &cmd_tx).await;
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
        let mut handler = session_handler.write().unwrap();
        handler.disconnect(&session_id);
    }

    let _ = cmd_tx.send(LuaCommand::OnDisconnect {
        session_id: session_id_str.clone(),
    });

    let _ = conn.close().await;
    tracing::info!("Connection closed: {} ({})", session_id_str, addr);
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
