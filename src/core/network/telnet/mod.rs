pub mod constants;
pub mod parser;
pub mod option;
pub mod codec;
pub mod connection;
pub mod mxp;
pub mod relay;

pub use constants::*;
pub use parser::{TelnetParser, TelnetEvent};
pub use option::{OptionNegotiator, NegotiationCommand, QState};
pub use codec::TelnetCodec;
pub use connection::{TelnetConnection, ConnectionId};
pub use mxp::{LineMode, MxpState};
/// Compatibility re-export. The struct moved to `core::session`, where it
/// belongs — it is a `Session` field and two transports fill it in — but
/// `telnet::ClientCapabilities` had callers and there is no reason to break
/// them.
pub use crate::core::session::ClientCapabilities;

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::config::driver_config::TelnetServerConfig;
use crate::core::network::tls;
use crate::core::{LuaCommand, SessionHandler};
use crate::error::{OxigeonError, Result};

/// What a telnet connection task needs from the driver.
///
/// The same shape as `websocket::WsDeps`, and for the same reason: everything
/// in it is independently clonable, so a listener needs no reference to the
/// `Driver` that started it.
#[derive(Clone)]
pub struct TelnetDeps {
    pub session_handler: Arc<RwLock<SessionHandler>>,
    pub cmd_tx: mpsc::UnboundedSender<LuaCommand>,
    /// `None` in the test harness, which has no address tally to forget.
    pub auth_worker: Option<crate::core::auth::AuthWorker>,
    pub input_buffer_bytes: usize,
    /// Whether to offer MXP. From `[servers.telnet].mxp`, and so the first
    /// field here that comes from the listener's own config rather than from
    /// `server_config` — MXP is a property of the wire, not of the game.
    pub mxp: bool,
}

/// Bind a telnet listener and serve clients until the process ends.
///
/// `telnet://` or `telnets://` depending on whether the config names a
/// certificate. Nothing below this line knows which: a finished TLS handshake
/// leaves an ordinary `AsyncRead + AsyncWrite`, and the IAC parser has never
/// cared what carried the bytes.
///
/// Returns the address actually bound, so callers — and tests, which bind port
/// 0 — know where it landed. Same shape as `websocket::serve` and
/// `debugger::dap::serve`; `Driver::run` starts listeners and does not own
/// their accept loops.
pub async fn serve(
    cfg: &TelnetServerConfig,
    section: &str,
    deps: TelnetDeps,
) -> Result<SocketAddr> {
    // Before binding: a certificate that does not load is a startup error, not
    // a port that quietly serves plaintext under a secure-sounding name.
    let acceptor = match crate::config::driver_config::tls_files(
        &cfg.cert_path,
        &cfg.key_path,
        section,
    )
    .map_err(OxigeonError::Config)?
    {
        Some((cert, key)) => Some(Arc::new(tls::acceptor_from_files(&cert, &key, cfg.cert_reload_seconds)?)),
        None => None,
    };

    let listener = TcpListener::bind((cfg.bind.as_str(), cfg.port)).await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let deps = deps.clone();
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        // On the connection's own task, so a peer that opens a
                        // socket to the TLS port and then stalls does not hold
                        // up everyone else's accept.
                        let stream = match tls::wrap(
                            stream,
                            acceptor.as_deref(),
                            tls::HANDSHAKE_TIMEOUT,
                        )
                        .await
                        {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::debug!("TLS handshake from {} failed: {}", peer, e);
                                return;
                            }
                        };
                        // `tokio::io::split` rather than `TcpStream::into_split`:
                        // it works on the TLS stream too, and one code path is
                        // worth a `BiLock` on a connection that is already
                        // dominated by syscalls.
                        let (reader, writer) = tokio::io::split(stream);
                        let conn = TelnetConnection::new(writer, peer, deps.mxp);
                        relay::run(conn, reader, peer, deps).await;
                    });
                }
                Err(e) => {
                    tracing::error!("Telnet accept failed: {}", e);
                    return;
                }
            }
        }
    });

    Ok(addr)
}
