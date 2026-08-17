use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::io::WriteHalf;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::codec::TelnetCodec;
use super::constants::*;
use super::mxp::{self, MxpState};
use super::option::OptionNegotiator;
use crate::core::network::MaybeTls;
use crate::core::session::ClientCapabilities;
use crate::error::Result;

/// Unique connection identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub Uuid);

impl ConnectionId {
    pub fn new() -> Self {
        ConnectionId(Uuid::new_v4())
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

/// A Telnet connection wrapping a stream that may or may not be encrypted.
///
/// `WriteHalf<MaybeTls>` rather than `OwnedWriteHalf`: `telnets://` is the same
/// protocol inside TLS, and a `TlsStream` cannot be `into_split()` the way a
/// `TcpStream` can. `tokio::io::split` works on anything that reads and writes,
/// at the cost of a `BiLock` the owned halves do not need — which is a real but
/// very small price for having one code path instead of two.
pub struct TelnetConnection {
    pub id: ConnectionId,
    pub address: SocketAddr,
    writer: Arc<Mutex<WriteHalf<MaybeTls>>>,
    pub negotiator: OptionNegotiator,
    pub capabilities: ClientCapabilities,
    /// Whether MXP is offered on this listener, and whether it is live.
    pub mxp: MxpState,
}

impl TelnetConnection {
    pub fn new(writer: WriteHalf<MaybeTls>, address: SocketAddr, offer_mxp: bool) -> Self {
        TelnetConnection {
            id: ConnectionId::new(),
            address,
            writer: Arc::new(Mutex::new(writer)),
            negotiator: OptionNegotiator::new(),
            capabilities: ClientCapabilities::default(),
            mxp: MxpState::new(offer_mxp),
        }
    }

    /// Send text **this crate wrote** — with CR/LF translation and IAC
    /// escaping, and nothing else.
    ///
    /// For anything that came from the mudlib use [`send_game_text`]. The split
    /// is not cosmetic: it is where the driver decides whether a string is
    /// trusted, and having the two named differently is what makes a reviewer
    /// notice a call that picked the wrong one.
    ///
    /// [`send_game_text`]: Self::send_game_text
    pub async fn send_text(&self, text: &str) -> Result<()> {
        let encoded = TelnetCodec::encode_text(text);
        self.send_raw(&encoded).await
    }

    /// Send text the **mudlib** produced, and therefore text a player may have
    /// had a hand in.
    ///
    /// Byte-identical to [`send_text`] until MXP is live on this connection.
    /// Once it is, line-mode sequences are stripped — see
    /// [`mxp::strip_line_modes`], which carries the explanation of why that
    /// single removal is what makes the rest of the stream safe. The text is
    /// otherwise untouched: the default line mode is LOCKED, so a `<` in a mob
    /// name is a `<` and not the start of a tag.
    ///
    /// [`send_text`]: Self::send_text
    pub async fn send_game_text(&self, text: &str) -> Result<()> {
        if self.mxp.is_enabled() {
            self.send_text(&mxp::strip_line_modes(text)).await
        } else {
            self.send_text(text).await
        }
    }

    /// Send a prompt: mudlib text with no trailing newline.
    ///
    /// Still `send_raw` underneath — a prompt exists precisely to leave the
    /// cursor where it is, so it must not gain a CRLF — but it goes through the
    /// same mode-sequence strip as the rest of the mudlib's output. A prompt is
    /// assembled from character data and is as reachable by a player as a room
    /// description is.
    pub async fn send_game_prompt(&self, bytes: &[u8]) -> Result<()> {
        if !self.mxp.is_enabled() {
            return self.send_raw(bytes).await;
        }
        // `send_prompt` builds these with `String::into_bytes`, so they are
        // valid UTF-8 by construction. Lossy anyway: it is total, free on the
        // happy path, and cannot panic in a connection task where a panic
        // reads to the player as a mysterious disconnect.
        let text = String::from_utf8_lossy(bytes);
        self.send_raw(mxp::strip_line_modes(&text).as_bytes()).await
    }

    /// Send raw bytes directly (for IAC sequences, GMCP, etc.)
    pub async fn send_raw(&self, data: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().await;
        writer.write_all(data).await?;
        Ok(())
    }

    /// Send a telnet negotiation command
    pub async fn send_negotiate(&self, verb: u8, option: u8) -> Result<()> {
        self.send_raw(&[IAC, verb, option]).await
    }

    /// Send a GMCP message
    pub async fn send_gmcp(&self, package: &str, json: Option<&str>) -> Result<()> {
        let bytes = TelnetCodec::encode_gmcp(package, json);
        self.send_raw(&bytes).await
    }

    /// Start echo masking (for password input)
    pub async fn start_echo(&mut self) -> Result<()> {
        if let Some(cmd) = self.negotiator.request_local_enable(OPT_ECHO) {
            self.send_raw(&cmd.to_bytes()).await?;
        }
        Ok(())
    }

    /// Stop echo masking
    pub async fn stop_echo(&mut self) -> Result<()> {
        if let Some(cmd) = self.negotiator.request_local_disable(OPT_ECHO) {
            self.send_raw(&cmd.to_bytes()).await?;
        }
        Ok(())
    }

    /// Send initial telnet negotiation offers
    pub async fn send_initial_negotiations(&mut self) -> Result<()> {
        // Suppress Go Ahead — enable full-duplex mode
        if let Some(cmd) = self.negotiator.request_local_enable(OPT_SGA) {
            self.send_raw(&cmd.to_bytes()).await?;
        }
        if let Some(cmd) = self.negotiator.request_remote_enable(OPT_SGA) {
            self.send_raw(&cmd.to_bytes()).await?;
        }

        // GMCP — MUD communication protocol
        if let Some(cmd) = self.negotiator.request_local_enable(OPT_GMCP) {
            self.send_raw(&cmd.to_bytes()).await?;
        }

        // MCCP2 — compression
        if let Some(cmd) = self.negotiator.request_local_enable(OPT_MCCP2) {
            self.send_raw(&cmd.to_bytes()).await?;
        }

        // Terminal type
        if let Some(cmd) = self.negotiator.request_remote_enable(OPT_TTYPE) {
            self.send_raw(&cmd.to_bytes()).await?;
        }

        // Window size
        if let Some(cmd) = self.negotiator.request_remote_enable(OPT_NAWS) {
            self.send_raw(&cmd.to_bytes()).await?;
        }

        // MXP — offered last, and only offered. A client that answers DONT, or
        // never answers at all, gets a session byte-identical to the one it got
        // before MXP existed: nothing changes about the output until the DO
        // arrives. Last in the burst so that a client which reads one round of
        // negotiation and then starts talking has already told us its terminal
        // and window size before markup enters the picture.
        if self.mxp.offered() {
            if let Some(cmd) = self.negotiator.request_local_enable(OPT_MXP) {
                self.send_raw(&cmd.to_bytes()).await?;
            }
        }

        Ok(())
    }

    /// Close the connection
    pub async fn close(&self) -> Result<()> {
        let mut writer = self.writer.lock().await;
        let _ = writer.shutdown().await;
        Ok(())
    }
}
