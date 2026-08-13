use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::mpsc;
use uuid::Uuid;
use serde_json::Value as JsonValue;

use super::capabilities::ClientCapabilities;

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
    /// **A prompt.** The name describes the telnet implementation — raw bytes,
    /// no CRLF appended — rather than the meaning, and there are now two
    /// transports that have to know which one is true.
    ///
    /// It really is only ever a prompt: this variant is constructed in exactly
    /// one place in the crate, inside the `send_prompt` efun, and both the
    /// testkit and the WebSocket envelope already read it that way. Renaming it
    /// to `Prompt(String)` would touch four sites and be strictly clearer;
    /// until then, this comment is the contract.
    Raw(Vec<u8>),
    Gmcp { package: String, data: JsonValue },
    StartEcho,
    StopEcho,
    Disconnect,
}

impl SessionOutput {
    /// Whether losing this message changes session state rather than just
    /// costing the player some text. Dropping `Disconnect` leaves a session
    /// that should be gone; dropping an echo toggle leaves a password visible.
    pub fn is_control(&self) -> bool {
        matches!(self, Self::Disconnect | Self::StartEcho | Self::StopEcho)
    }
}

/// What the player sees where output went missing.
const TRUNCATION_MARKER: &str = "\r\n[... output truncated — you are receiving text faster \
                                 than your connection can take it ...]\r\n";

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

    /// Messages dropped since the player was last told about it. Atomic
    /// because every efun reaches a `Session` through a read lock on
    /// `SessionHandler` and so only ever holds `&Session`.
    dropped_pending: AtomicU64,
    /// Every message this session has ever lost, for `session_info`.
    dropped_total: AtomicU64,
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
            dropped_pending: AtomicU64::new(0),
            dropped_total: AtomicU64::new(0),
        }
    }

    /// Queue output for this session without blocking the caller.
    ///
    /// Every send site used to be `let _ = output_tx.try_send(..)`. Against a
    /// 64-slot channel that meant a player on a slow link, or any burst over
    /// 64 messages, silently lost text: no log, no counter, no marker. It
    /// presented as "the MUD ate my output" and was close to impossible to
    /// reproduce on demand.
    ///
    /// Blocking is not an option — most callers are on the Lua thread, which
    /// is the whole game — so a full channel still drops. But the loss is now
    /// counted, logged, and shown to the player as soon as there is room.
    ///
    /// Returns whether the message was queued.
    pub fn try_send(&self, out: SessionOutput) -> bool {
        let control = out.is_control();
        self.flush_drop_notice();

        match self.output_tx.try_send(out) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // The connection task is gone. Not a drop worth reporting —
                // there is no one left to report it to.
                false
            }
            Err(mpsc::error::TrySendError::Full(out)) => {
                let total = self.dropped_total.fetch_add(1, Ordering::Relaxed) + 1;
                self.dropped_pending.fetch_add(1, Ordering::Relaxed);
                if control {
                    // Losing a Disconnect or an echo toggle is not cosmetic:
                    // the session stays connected, or the player's password
                    // stays visible. Always loud.
                    tracing::error!(
                        "session {}: dropped control message {:?} — output channel full",
                        self.id,
                        out
                    );
                } else if total == 1 || total % 100 == 0 {
                    tracing::warn!(
                        "session {} ({}): output channel full, {} message(s) dropped so far",
                        self.id,
                        self.address,
                        total
                    );
                }
                false
            }
        }
    }

    /// Tell the player about anything lost since the last notice, if there is
    /// now room to say so. Best effort by construction: if the channel is
    /// still full the count simply stays pending until it is not.
    fn flush_drop_notice(&self) {
        if self.dropped_pending.load(Ordering::Relaxed) == 0 {
            return;
        }
        if self
            .output_tx
            .try_send(SessionOutput::Text(TRUNCATION_MARKER.to_string()))
            .is_ok()
        {
            self.dropped_pending.store(0, Ordering::Relaxed);
        }
    }

    /// Total messages this session has lost to a full output channel.
    pub fn dropped_output(&self) -> u64 {
        self.dropped_total.load(Ordering::Relaxed)
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
