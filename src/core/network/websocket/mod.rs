//! WebSocket listener.
//!
//! A second transport onto the same sessions. Everything above the connection
//! task was already transport-neutral — `Session` carries a `protocol` string,
//! `SessionHandler` is keyed by a UUID, and every output efun resolves a
//! session id to an `mpsc::Sender<SessionOutput>` without knowing what is on
//! the far end — so this module adds a way in and changes nothing else. A
//! `send()` from Lua reaches a browser and a telnet client by the same path.
//!
//! Modelled on `debugger::dap::serve`: the listener owns its own accept loop
//! and `Driver::run` only starts it. Adding an arm to the driver's `select!`
//! would tie this listener's lifetime to the telnet one and make every future
//! transport grow that loop.

pub mod connection;
pub mod protocol;

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::config::WebSocketServerConfig;
use crate::core::{LuaCommand, SessionHandler};

/// What a connection task needs from the driver.
///
/// A struct rather than six positional parameters: `handle_connection`'s
/// seven-argument signature is already at the edge of readable and this one
/// would be worse. Everything in it is independently clonable, which is why
/// none of this needs `&Driver`.
#[derive(Clone)]
pub struct WsDeps {
    pub session_handler: Arc<RwLock<SessionHandler>>,
    pub cmd_tx: mpsc::UnboundedSender<LuaCommand>,
    /// `None` in the test harness, which has no address tally to forget.
    /// Mirrors `EfunContext.auth_worker`.
    pub auth_worker: Option<crate::core::auth::AuthWorker>,
    /// `limits.input_buffer_bytes`, applied per inbound `input` frame.
    pub input_buffer_bytes: usize,
}

/// The per-connection knobs, lifted out of the config so a connection task does
/// not carry a whole `WebSocketServerConfig` it would only read three fields of.
#[derive(Clone)]
pub struct WsRuntime {
    pub max_frame_bytes: usize,
    pub ping_interval_secs: u64,
    pub missed_pongs: u32,
    pub input_buffer_bytes: usize,
    /// Browser origins permitted to open a socket. Empty accepts any.
    pub allowed_origins: Arc<Vec<String>>,
}

/// Bind the WebSocket listener and serve clients until the process ends.
///
/// `ws://` or `wss://` depending on whether the config names a certificate;
/// nothing below this line knows the difference, because a finished TLS
/// handshake leaves an ordinary `AsyncRead + AsyncWrite` behind.
///
/// Returns the address actually bound, so callers — and tests, which bind port
/// 0 — know where it landed.
pub async fn serve(
    cfg: &WebSocketServerConfig,
    deps: WsDeps,
) -> crate::error::Result<SocketAddr> {
    // Built before binding: a certificate that does not load is a startup
    // error, never a port that quietly serves plaintext under a secure name.
    let acceptor = match crate::config::driver_config::tls_files(
        &cfg.cert_path,
        &cfg.key_path,
        "websocket_tls",
    )
    .map_err(crate::error::OxigeonError::Config)?
    {
        Some((cert, key)) => Some(crate::core::network::tls::acceptor_from_files(&cert, &key, cfg.cert_reload_seconds)?),
        None => None,
    };
    let listener = TcpListener::bind((cfg.bind.as_str(), cfg.port)).await?;
    let addr = listener.local_addr()?;

    let runtime = WsRuntime {
        max_frame_bytes: cfg.max_frame_bytes,
        ping_interval_secs: cfg.ping_interval_secs,
        missed_pongs: cfg.missed_pongs,
        input_buffer_bytes: deps.input_buffer_bytes,
        allowed_origins: Arc::new(cfg.allowed_origins.clone()),
    };

    let acceptor = acceptor.map(std::sync::Arc::new);

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let deps = deps.clone();
                    let acceptor = acceptor.clone();
                    let runtime = runtime.clone();
                    tokio::spawn(async move {
                        // The handshake happens on the connection's own task,
                        // not in the accept loop: a peer that opens a socket to
                        // a TLS port and then stalls must not hold up everyone
                        // else's accept.
                        let stream = match crate::core::network::tls::wrap(
                            stream,
                            acceptor.as_deref(),
                            crate::core::network::tls::HANDSHAKE_TIMEOUT,
                        )
                        .await
                        {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::debug!("TLS handshake from {} failed: {}", peer, e);
                                return;
                            }
                        };
                        connection::run(stream, peer, deps, runtime).await;
                    });
                }
                Err(e) => {
                    // An accept error here is the listener itself failing, not
                    // one client — the same treatment the telnet loop gives it,
                    // except that this task has nothing else to do afterwards.
                    tracing::error!("WebSocket accept failed: {}", e);
                    return;
                }
            }
        }
    });

    Ok(addr)
}

