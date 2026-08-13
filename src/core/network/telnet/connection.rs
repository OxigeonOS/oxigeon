use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::io::WriteHalf;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::codec::TelnetCodec;
use super::constants::*;
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
}

impl TelnetConnection {
    pub fn new(writer: WriteHalf<MaybeTls>, address: SocketAddr) -> Self {
        TelnetConnection {
            id: ConnectionId::new(),
            address,
            writer: Arc::new(Mutex::new(writer)),
            negotiator: OptionNegotiator::new(),
            capabilities: ClientCapabilities::default(),
        }
    }

    /// Send text to the client with CR/LF translation and IAC escaping.
    pub async fn send_text(&self, text: &str) -> Result<()> {
        let encoded = TelnetCodec::encode_text(text);
        self.send_raw(&encoded).await
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

        Ok(())
    }

    /// Close the connection
    pub async fn close(&self) -> Result<()> {
        let mut writer = self.writer.lock().await;
        let _ = writer.shutdown().await;
        Ok(())
    }
}
