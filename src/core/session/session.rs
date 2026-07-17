use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Instant;
use tokio::sync::mpsc;
use uuid::Uuid;
use serde_json::Value as JsonValue;

use crate::core::network::telnet::ClientCapabilities;

/// Unique session identifier (UUID-based, globally unique)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(Uuid);

impl SessionId {
    pub fn new() -> Self {
        SessionId(Uuid::new_v4())
    }

    pub fn as_str(&self) -> String {
        self.0.to_string()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::str::FromStr for SessionId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(SessionId(Uuid::parse_str(s)?))
    }
}

/// The lifecycle state of a session
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    /// TCP connected, no authentication yet
    Connected,
    /// In the login/registration flow (Lua drives this)
    Authenticating,
    /// Authenticated, associated with an Account
    Authenticated { account_id: i64 },
    /// In-game, controlling a character
    Playing { account_id: i64, character_id: i64 },
}

impl SessionState {
    pub fn name(&self) -> &str {
        match self {
            SessionState::Connected => "connected",
            SessionState::Authenticating => "authenticating",
            SessionState::Authenticated { .. } => "authenticated",
            SessionState::Playing { .. } => "playing",
        }
    }

    pub fn account_id(&self) -> Option<i64> {
        match self {
            SessionState::Authenticated { account_id } => Some(*account_id),
            SessionState::Playing { account_id, .. } => Some(*account_id),
            _ => None,
        }
    }

    pub fn character_id(&self) -> Option<i64> {
        match self {
            SessionState::Playing { character_id, .. } => Some(*character_id),
            _ => None,
        }
    }
}

/// Messages sent from the driver to a connection task
#[derive(Debug)]
pub enum SessionOutput {
    Text(String),
    Raw(Vec<u8>),
    Gmcp { package: String, data: JsonValue },
    StartEcho,
    StopEcho,
    Disconnect,
}

/// A session tracks a single client connection to the server.
/// Non-persistent — exists only while the client is connected.
pub struct Session {
    pub id: SessionId,
    /// Protocol name ("telnet", "websocket", etc.)
    pub protocol: String,
    pub address: SocketAddr,
    pub connected_at: Instant,
    pub state: SessionState,
    pub capabilities: ClientCapabilities,
    /// Channel to send output to the connection task
    pub output_tx: mpsc::Sender<SessionOutput>,
    /// Cached permission strings loaded at enter_game time.
    /// The sentinel "**superuser**" means account.is_admin=true and bypasses all checks.
    pub permissions: HashSet<String>,
}

impl Session {
    pub fn new(
        protocol: String,
        address: SocketAddr,
        output_tx: mpsc::Sender<SessionOutput>,
    ) -> Self {
        Session {
            id: SessionId::new(),
            protocol,
            address,
            connected_at: Instant::now(),
            state: SessionState::Connected,
            capabilities: ClientCapabilities::default(),
            output_tx,
            permissions: HashSet::new(),
        }
    }

    /// Send text to this session
    pub async fn send_text(&self, text: &str) {
        let _ = self.output_tx.send(SessionOutput::Text(text.to_string())).await;
    }

    /// Send GMCP to this session
    pub async fn send_gmcp(&self, package: &str, data: JsonValue) {
        let _ = self.output_tx.send(SessionOutput::Gmcp {
            package: package.to_string(),
            data,
        }).await;
    }

    pub fn is_authenticated(&self) -> bool {
        self.state.account_id().is_some()
    }
}
