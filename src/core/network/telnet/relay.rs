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
use super::{TelnetCodec, TelnetConnection, TelnetEvent, TelnetParser};
use crate::core::lock::RwLockExt;
use crate::core::network::MaybeTls;
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
    let TelnetDeps { session_handler, cmd_tx, auth_worker, input_buffer_bytes: max_buf } = deps;
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
async fn handle_negotiation(
    conn: &mut TelnetConnection,
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
        let _ = conn.send_gmcp("Core.Hello", Some(&format!(r#"{{"client":"Oxigeon","version":"{}"}}"#, env!("CARGO_PKG_VERSION")))).await;
    }

    // Note MCCP2 negotiation
    if option == OPT_MCCP2 && verb == DO {
        conn.capabilities.mccp2_supported = true;
    }
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
