// Nothing about a wire protocol is imported here any more. The telnet parser,
// the codec and the option constants left with `handle_connection`, which is in
// `network::telnet::relay` where it always belonged — see that module's header.
// A driver that knows what an IAC byte is has a listener living in the wrong
// file.
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use crate::core::lock::RwLockExt;

use crate::config::{DriverConfig, ServerConfig};
use crate::config::driver_config::DatabaseBackend;
use crate::config::PermissionConfig;
use crate::core::{SessionHandler, ScriptEngine, EfunContext};
use crate::core::logging::{GameLogger, utc_now};
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

    /// Start every configured listener, then wait for Ctrl+C.
    ///
    /// Each listener owns its own accept loop — the shape `dap::serve`
    /// established and every transport now follows — so this function starts
    /// things and then has exactly one job left, which is to notice a shutdown
    /// and flush the mudlib.
    pub async fn run(&self) -> Result<()> {
        // Debug adapter first, so an editor can attach while the game is idle.
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
                            "debug adapter bound to {} — it grants unauthenticated control                              of the game VM and must not be reachable off-host",
                            addr.ip()
                        );
                    }
                }
                Err(e) => tracing::error!("Failed to start debug adapter: {}", e),
            }
        }

        let mut listeners = 0usize;

        // ── Telnet, plaintext and TLS ─────────────────────────
        for (section, cfg) in [
            ("telnet", self.driver_config.servers.telnet.as_ref()),
            ("telnet_tls", self.driver_config.servers.telnet_tls.as_ref()),
        ] {
            let Some(cfg) = cfg.filter(|c| c.enabled) else { continue };
            let deps = crate::core::network::telnet::TelnetDeps {
                session_handler: self.session_handler.clone(),
                cmd_tx: self.script_engine.cmd_tx.clone(),
                auth_worker: Some(self.auth_worker.clone()),
                input_buffer_bytes: self.server_config.limits.input_buffer_bytes,
                mxp: cfg.mxp,
            };
            match crate::core::network::telnet::serve(cfg, section, deps).await {
                Ok(addr) => {
                    listeners += 1;
                    let scheme = if cfg.cert_path.is_some() { "telnets" } else { "telnet" };
                    tracing::info!("Telnet server listening on {}://{}", scheme, addr);
                }
                // Fatal rather than logged: a `[servers.*]` block that was
                // asked for and did not come up is a port the operator
                // believes is open. Starting anyway is how you discover at
                // 3am that nobody could connect.
                Err(e) => {
                    return Err(OxigeonError::Config(format!(
                        "[servers.{section}] failed to start: {e}"
                    )))
                }
            }
        }

        // ── WebSocket, plaintext and TLS ──────────────────────
        for (section, cfg) in [
            ("websocket", self.driver_config.servers.websocket.as_ref()),
            ("websocket_tls", self.driver_config.servers.websocket_tls.as_ref()),
        ] {
            let Some(cfg) = cfg.filter(|c| c.enabled) else { continue };
            let deps = crate::core::network::websocket::WsDeps {
                session_handler: self.session_handler.clone(),
                cmd_tx: self.script_engine.cmd_tx.clone(),
                auth_worker: Some(self.auth_worker.clone()),
                input_buffer_bytes: self.server_config.limits.input_buffer_bytes,
            };
            match crate::core::network::websocket::serve(cfg, deps).await {
                Ok(addr) => {
                    listeners += 1;
                    let scheme = if cfg.cert_path.is_some() { "wss" } else { "ws" };
                    tracing::info!("WebSocket server listening on {}://{}", scheme, addr);
                }
                Err(e) => {
                    return Err(OxigeonError::Config(format!(
                        "[servers.{section}] failed to start: {e}"
                    )))
                }
            }
        }

        // A server with no listener accepts nothing. Disabling telnet used to
        // `return Ok(())` here, which `main` reports as a clean exit — so the
        // operator saw a successful startup banner and a dead port, and
        // `shutdown_lua` never ran either, so the mudlib was never asked to
        // flush on the way out.
        if listeners == 0 {
            return Err(OxigeonError::Config(
                "no listener is enabled — enable one of [servers.telnet],                  [servers.telnet_tls], [servers.websocket] or [servers.websocket_tls]"
                    .into(),
            ));
        }

        tracing::info!(
            "Oxigeon v{} started — {} listener{} up",
            env!("CARGO_PKG_VERSION"),
            listeners,
            if listeners == 1 { "" } else { "s" }
        );

        // Compute deadlines are watched by a thread the bridge owns, not from
        // here — see `ComputeBridge::spawn_watchdog`.
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutdown signal received");
        {
            let handler = self.session_handler.read_recover();
            handler.broadcast("

Server shutting down. Goodbye!

");
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
