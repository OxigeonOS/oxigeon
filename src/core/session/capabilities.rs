//! What the driver knows about the client on the other end of a session.
//!
//! This lives beside `Session` rather than inside `network::telnet` — where it
//! was first written — because it is a field on `Session` and two transports
//! now fill it in. `core::session` importing from `core::network::telnet` had
//! the dependency backwards, and the next person to add a field would have
//! taken the location as permission to make it telnet-specific.
//!
//! `mccp2_supported` and `mccp2_active` stay on the struct even though only
//! telnet can ever set them. `get_session` marshals this shape straight to Lua,
//! and the mudlib must not have to branch on transport to read a window width.

use std::sync::{Arc, RwLock};

use super::{SessionHandler, SessionId};
use crate::core::lock::RwLockExt;

/// Client capabilities, discovered by whatever the transport uses to ask.
///
/// Telnet fills this in over several negotiation round trips; a WebSocket
/// client announces it in one `hello` frame, and may send another whenever its
/// window changes.
#[derive(Debug, Clone, Default)]
pub struct ClientCapabilities {
    pub terminal_type: Option<String>,
    pub window_width: Option<u16>,
    pub window_height: Option<u16>,
    pub mccp2_supported: bool,
    pub mccp2_active: bool,
    pub gmcp_supported: bool,
    pub gmcp_packages: Vec<String>,

    /// The client accepted telnet option 91 and the driver has locked the
    /// stream to LOCKED mode.
    ///
    /// **This, and not a client name, is the flag to branch on.** A client that
    /// never answers `<VERSION>` — most do not — still parses markup perfectly
    /// well, so gating rich output on knowing who is out there would silently
    /// disable the feature for the majority.
    pub mxp_supported: bool,
    /// `MXP=` from the client's `<VERSION>` reply: the level of the spec it
    /// implements, e.g. `"0.4"`. `None` if it never answered.
    pub mxp_version: Option<String>,
    /// `CLIENT` and `VERSION` from the same reply, joined — `"mushclient 5.06"`.
    ///
    /// One string because the only question anyone asks of it is "who is it",
    /// and deliberately *not* `terminal_type`: TTYPE answers a different
    /// question and a client is entitled to give two different answers.
    pub mxp_client: Option<String>,
    /// The `+tag` / `-tag` tokens from `<SUPPORTS>`, signs kept.
    ///
    /// Not interpreted here. Which tags matter is a question about what a game
    /// wants to emit, and a driver-side allowlist would be a second place to
    /// edit every time the answer changed.
    pub mxp_supports: Vec<String>,
}

impl ClientCapabilities {
    /// What a WebSocket client is assumed to be until it says otherwise.
    ///
    /// GMCP is **on** by default here, unlike telnet where it is on only after
    /// a negotiation round trip. A client that did not want GMCP would not have
    /// connected to a JSON envelope, and the two failure modes are not
    /// symmetric: guess "off" and every `gmcp_d` sender returns at its first
    /// guard while the link still looks healthy — which is precisely the bug
    /// `publish_capabilities` below exists to record.
    ///
    /// 80 columns matches the mudlib's own `Player.DEFAULT_WRAP_WIDTH`, so a
    /// client that never sends `hello` gets the wrap the mudlib would have
    /// chosen for itself rather than a second, different guess.
    pub fn for_websocket() -> Self {
        ClientCapabilities {
            terminal_type: Some("websocket".to_string()),
            window_width: Some(80),
            window_height: Some(24),
            gmcp_supported: true,
            ..Default::default()
        }
    }
}

/// Copy what a transport discovered onto the **Session**.
///
/// Negotiation writes to the connection's own `capabilities`; the mudlib reads
/// `Session.capabilities`, through `get_session`. They are two structs on two
/// objects and nothing joined them, so `Session.capabilities` sat at
/// `Default::default()` for the life of every session that had ever connected.
///
/// The consequences were all silent. `gmcp_d` guards every one of its four
/// senders on `sess.gmcp_supported`, so **no GMCP was ever pushed to any
/// client** — the TUI's Room.Info, Char.Vitals and Effects panes could not
/// populate, and the `Core.Hello` a client does receive comes straight from the
/// transport and never touches Lua, which is what made the link look healthy.
/// `window_width` was nil too, so output was wrapped to a default regardless of
/// the terminal's real size.
///
/// Telnet calls this after every negotiation and subnegotiation rather than
/// once at the end: NAWS arrives again on every resize, and TTYPE can arrive
/// well after the first GMCP message. WebSocket calls it once before
/// `on_connect` and again on every `hello`, for the same reason.
pub fn publish_capabilities(
    session_handler: &Arc<RwLock<SessionHandler>>,
    session_id: SessionId,
    caps: &ClientCapabilities,
) {
    let mut handler = session_handler.write_recover();
    if let Some(session) = handler.get_mut(&session_id) {
        session.capabilities = caps.clone();
    }
}
