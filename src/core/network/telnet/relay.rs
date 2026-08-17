//! One telnet client, for its entire lifetime.
//!
//! This lived in `driver.rs` until `telnets://` arrived and made it clear it
//! never belonged there: it is the reason the driver imported `TelnetParser`,
//! `TelnetCodec` and the option constants, and the reason negotiation *policy*
//! — the `Core.Hello` push, the inbound-GMCP dispatch — sat two modules away
//! from the state machine it drives. Nothing about it is the driver's business.
//!
//! Plaintext and TLS run the identical code. By the time a stream reaches here
//! the handshake is over and what is left is bytes, which is the whole reason
//! `telnets` cost a type change rather than a second implementation.

use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, ReadHalf};
use tokio::sync::mpsc;

use super::constants::*;
use super::mxp;
use super::option::OptionNegotiator;
use super::{TelnetCodec, TelnetConnection, TelnetEvent, TelnetParser};
use crate::core::lock::RwLockExt;
use crate::core::network::MaybeTls;
use crate::core::render;
use crate::core::session::publish_capabilities;
use crate::core::{LuaCommand, Session, SessionOutput};

use super::TelnetDeps;

/// Handle a single client connection for its entire lifetime.
/// Implements the full bidirectional relay loop:
///   TCP read → Telnet parse → Lua on_input
///   Lua send → SessionOutput → TCP write
pub(super) async fn run(
    mut conn: TelnetConnection,
    mut reader: ReadHalf<MaybeTls>,
    addr: SocketAddr,
    deps: TelnetDeps,
) {
    // `mxp` was consumed by `TelnetConnection::new` before this task started —
    // it decides what the opening negotiation burst offers, and the connection
    // owns the answer from there.
    let TelnetDeps { session_handler, cmd_tx, auth_worker, input_buffer_bytes: max_buf, mxp: _ } = deps;
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
    // `on_mxp_ready` fires once, on the client's first handshake reply. A
    // client answers both `<VERSION>` and `<SUPPORT>`, and may answer a
    // narrowed `<SUPPORT>` again later.
    let mut mxp_announced = false;

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

                            // A client that was asked `<VERSION>` or `<SUPPORT>`
                            // answers here, on the ordinary input stream, on a
                            // secure line of its own. It is a protocol reply and
                            // not something the player typed: dispatching it
                            // makes the mudlib say "Huh?" at a client for doing
                            // exactly what it was told, and drops the client's
                            // version string into whatever the login state
                            // machine happened to be reading.
                            if conn.mxp.is_enabled() {
                                if let Some(reply) = mxp::parse_reply(&line) {
                                    mxp::apply(&reply, &mut conn.capabilities);
                                    if !mxp_announced {
                                        mxp_announced = true;
                                        let _ = cmd_tx.send(LuaCommand::OnMxpReady {
                                            session_id: session_id_str.clone(),
                                        });
                                    }
                                    // The rule this file already lives by, and
                                    // the one place it is easy to miss: the
                                    // calls above are on the Negotiate and
                                    // Subnegotiation arms, and this is neither.
                                    // A capability the network layer discovered
                                    // is not state until something copies it to
                                    // where the game looks.
                                    publish_capabilities(&session_handler, session_id, &conn.capabilities);
                                    continue;
                                }
                            }

                            // Whatever else this is, it is not allowed to carry
                            // a line-mode sequence. A player who types one is
                            // aiming it at the *other* players it will be
                            // echoed to; the outbound strip in `send_game_text`
                            // is what actually stops that, and this keeps it
                            // from being stored in a name or a description on
                            // the way past.
                            //
                            // Matched rather than `into_owned`: a line with no
                            // escape character in it — which is every line
                            // anybody types — is handed on as it stands rather
                            // than copied to say nothing changed.
                            let line = match mxp::strip_line_modes(&line) {
                                std::borrow::Cow::Borrowed(_) => line,
                                std::borrow::Cow::Owned(stripped) => stripped,
                            };

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
                        let _ = conn.send_game_text(&text).await;
                    }
                    Some(SessionOutput::Raw(bytes)) => {
                        let _ = conn.send_game_prompt(&bytes).await;
                    }
                    // The rendering is chosen here, at the transport, because
                    // this is the only place that knows whether the far end has
                    // an MXP parser. `send_text` and not `send_game_text`:
                    // every byte of this string was written by `render`, whose
                    // whole job is that it is safe.
                    Some(SessionOutput::Rich(line)) => {
                        let s = if conn.mxp.is_enabled() {
                            render::to_mxp(&line)
                        } else {
                            render::to_text(&line)
                        };
                        let _ = conn.send_text(&s).await;
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
    if let Some(auth) = &auth_worker {
        auth.forget(Some(addr.ip()));
    }

    let _ = conn.close().await;
    tracing::info!("Connection closed: {} ({})", session_id_str, addr);
}

/// Handle a Telnet negotiation event.
///
/// The decision is [`on_negotiate`] below; what is left here is the `await`.
async fn handle_negotiation(
    conn: &mut TelnetConnection,
    _session_id: &str,
    verb: u8,
    option: u8,
    _cmd_tx: &tokio::sync::mpsc::UnboundedSender<LuaCommand>,
) {
    let out = on_negotiate(
        verb,
        option,
        &mut conn.negotiator,
        &mut conn.capabilities,
        &mut conn.mxp,
    );
    if !out.is_empty() {
        let _ = conn.send_raw(&out).await;
    }
}

/// What to write in answer to one `IAC WILL/WONT/DO/DONT <option>`.
///
/// **A pure function over the connection's negotiation state**, returning bytes
/// rather than writing them — the same shape as the TUI client's own `handle`,
/// and for the same reason. This is where negotiation *policy* lives, which is
/// the thing this module's header says belongs here; until it was a free
/// function the only way to reach it was through a socket, which is why this
/// file had no tests at all and why nobody had noticed the GMCP arm re-pushing
/// `Core.Hello` on every repeated offer.
fn on_negotiate(
    verb: u8,
    option: u8,
    negotiator: &mut OptionNegotiator,
    caps: &mut crate::core::ClientCapabilities,
    mxp_state: &mut mxp::MxpState,
) -> Vec<u8> {
    let mut out = Vec::new();

    let response = match verb {
        WILL => negotiator.receive_will(option).0,
        WONT => negotiator.receive_wont(option).0,
        DO => negotiator.receive_do(option).0,
        DONT => negotiator.receive_dont(option).0,
        _ => None,
    };
    if let Some(cmd) = response {
        out.extend(cmd.to_bytes());
    }

    match option {
        // GMCP. This fires on every DO, including a repeat, so a client that
        // re-offers gets a second `Core.Hello`. Preserved as it was rather than
        // quietly changed: it is harmless — GMCP is out of band and a client
        // that asks twice can read the answer twice — and fixing it here would
        // bury a behaviour change in an unrelated feature. It is now at least
        // visible, and has a test.
        OPT_GMCP if verb == DO || verb == WILL => {
            caps.gmcp_supported = true;
            out.extend(TelnetCodec::encode_gmcp(
                "Core.Hello",
                Some(&format!(
                    r#"{{"client":"Oxigeon","version":"{}"}}"#,
                    env!("CARGO_PKG_VERSION")
                )),
            ));
        }

        OPT_MCCP2 if verb == DO => caps.mccp2_supported = true,

        // MXP. `DO` is the negotiated direction — the server offers WILL and
        // the client answers DO — but several clients send `WILL` meaning the
        // same thing, and refusing a peer that is trying to agree helps nobody.
        //
        // Unlike the GMCP arm this is idempotent, because `MxpState::enable`
        // returns nothing the second time: a repeated `DO` must not re-lock the
        // stream and re-ask the handshake in the middle of a page the player is
        // reading.
        OPT_MXP if verb == DO || verb == WILL => out.extend(mxp::on_accept(caps, mxp_state)),
        OPT_MXP if verb == DONT || verb == WONT => mxp::on_refuse(caps, mxp_state),

        _ => {}
    }

    out
}


/// Handle a Telnet subnegotiation event.
async fn handle_subnegotiation(
    conn: &mut TelnetConnection,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ClientCapabilities;

    fn negotiate(verb: u8, option: u8) -> (Vec<u8>, ClientCapabilities, mxp::MxpState) {
        let mut neg = OptionNegotiator::new();
        let mut caps = ClientCapabilities::default();
        let mut state = mxp::MxpState::new(true);
        let out = on_negotiate(verb, option, &mut neg, &mut caps, &mut state);
        (out, caps, state)
    }

    #[test]
    fn accepting_mxp_records_the_capability_and_starts_the_protocol() {
        let (out, caps, state) = negotiate(DO, OPT_MXP);
        assert!(caps.mxp_supported);
        assert!(state.is_enabled());
        assert!(out.windows(5).any(|w| w == [IAC, SB, OPT_MXP, IAC, SE]));
        assert!(String::from_utf8_lossy(&out).contains("\x1b[7z"));
    }

    /// The direction is server-WILL / client-DO, but clients send `WILL`
    /// meaning agreement and there is nothing to gain by refusing them.
    #[test]
    fn a_client_that_says_will_instead_of_do_is_taken_at_its_word() {
        let (_, caps, state) = negotiate(WILL, OPT_MXP);
        assert!(caps.mxp_supported);
        assert!(state.is_enabled());
    }

    #[test]
    fn a_repeated_offer_does_not_restart_mxp() {
        let mut neg = OptionNegotiator::new();
        let mut caps = ClientCapabilities::default();
        let mut state = mxp::MxpState::new(true);

        let first = on_negotiate(DO, OPT_MXP, &mut neg, &mut caps, &mut state);
        let second = on_negotiate(DO, OPT_MXP, &mut neg, &mut caps, &mut state);
        assert!(!first.is_empty());
        assert!(
            !String::from_utf8_lossy(&second).contains("\x1b[7z"),
            "a second DO must not re-lock the stream and re-ask the handshake"
        );
    }

    #[test]
    fn refusing_mxp_clears_the_capability() {
        let mut neg = OptionNegotiator::new();
        let mut caps = ClientCapabilities::default();
        let mut state = mxp::MxpState::new(true);

        on_negotiate(DO, OPT_MXP, &mut neg, &mut caps, &mut state);
        on_negotiate(DONT, OPT_MXP, &mut neg, &mut caps, &mut state);
        assert!(!caps.mxp_supported);
        assert!(!state.is_enabled());
    }

    /// Lifted out of `handle_negotiation` unchanged. It had no test before, and
    /// the only way to reach it was a socket.
    #[test]
    fn gmcp_acceptance_records_the_flag_and_pushes_core_hello() {
        let (out, caps, _) = negotiate(DO, OPT_GMCP);
        assert!(caps.gmcp_supported);
        assert!(String::from_utf8_lossy(&out).contains("Core.Hello"));
    }

    #[test]
    fn mccp2_acceptance_records_the_flag() {
        let (_, caps, _) = negotiate(DO, OPT_MCCP2);
        assert!(caps.mccp2_supported);
    }

    #[test]
    fn an_unrelated_option_gets_a_reply_and_grants_no_capability() {
        let (out, caps, _) = negotiate(WILL, OPT_NAWS);
        assert_eq!(out, vec![IAC, DO, OPT_NAWS]);
        assert!(!caps.mxp_supported && !caps.gmcp_supported && !caps.mccp2_supported);
    }
}
