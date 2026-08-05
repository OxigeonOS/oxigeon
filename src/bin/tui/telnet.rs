//! The client half of `src/core/network/telnet/`.
//!
//! `TelnetParser` is a pure byte state machine — it does not care which end of
//! the connection it is on — so the protocol decoding here is the driver's own
//! code, and only the negotiation *answers* are new.
//!
//! The server offers `WILL SGA`, `DO SGA`, `WILL GMCP`, `WILL MCCP2`, `DO TTYPE`
//! and `DO NAWS` on connect (`connection.rs::send_initial_negotiations`), then
//! `WILL ECHO` / `WONT ECHO` around the password prompt.

use std::collections::HashSet;

use oxigeon::core::network::telnet::codec::TelnetCodec;
use oxigeon::core::network::telnet::constants::*;
use oxigeon::core::network::telnet::parser::{TelnetEvent, TelnetParser};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::ansi::AnsiDecoder;
use crate::app::{Action, AppEvent};

/// What we tell the server we understand. `gmcp_d.wants()` gates every outbound
/// package on this list — a module covers its packages, so `Char` buys
/// `Char.Vitals`, `Char.Status` and `Char.Effects`.
const SUPPORTS: &str = r#"["Char 1","Room 1","Core 1"]"#;

const TERMINAL_TYPE: &str = "OXIGEON-TUI";

/// TTYPE subnegotiation commands (RFC 1091).
const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;

pub async fn run(
    addr: String,
    events: UnboundedSender<AppEvent>,
    mut actions: UnboundedReceiver<Action>,
) {
    let stream = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            let _ = events.send(AppEvent::TelnetDown(format!("{}: {}", addr, e)));
            return;
        }
    };
    let _ = stream.set_nodelay(true);
    let _ = events.send(AppEvent::TelnetUp);

    let (mut reader, mut writer) = stream.into_split();
    let mut parser = TelnetParser::new();
    let mut ansi = AnsiDecoder::new();
    let mut answered: HashSet<(u8, u8)> = HashSet::new();
    let mut size = (100u16, 30u16);
    let mut buf = [0u8; 8192];

    loop {
        tokio::select! {
            read = reader.read(&mut buf) => {
                let n = match read {
                    Ok(0) => {
                        let _ = events.send(AppEvent::TelnetDown("server closed".into()));
                        return;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        let _ = events.send(AppEvent::TelnetDown(e.to_string()));
                        return;
                    }
                };
                let mut out: Vec<u8> = Vec::new();
                for event in parser.feed_bytes(&buf[..n]) {
                    handle(event, &events, &mut ansi, &mut answered, &mut out, size);
                }
                if !out.is_empty() && writer.write_all(&out).await.is_err() {
                    let _ = events.send(AppEvent::TelnetDown("write failed".into()));
                    return;
                }
            }
            action = actions.recv() => {
                let bytes = match action {
                    None => return,
                    Some(Action::Send(line)) => {
                        // encode_text handles LF → CRLF and escapes 0xFF as IAC IAC,
                        // so a player typing a high byte cannot inject a command.
                        TelnetCodec::encode_text(&format!("{}\n", line))
                    }
                    Some(Action::Naws(w, h)) => {
                        size = (w, h);
                        naws(w, h)
                    }
                    Some(Action::Dap(..)) => continue,
                };
                if writer.write_all(&bytes).await.is_err() {
                    let _ = events.send(AppEvent::TelnetDown("write failed".into()));
                    return;
                }
            }
        }
    }
}

fn handle(
    event: TelnetEvent,
    events: &UnboundedSender<AppEvent>,
    ansi: &mut AnsiDecoder,
    answered: &mut HashSet<(u8, u8)>,
    out: &mut Vec<u8>,
    size: (u16, u16),
) {
    match event {
        TelnetEvent::Data(bytes) => {
            for line in ansi.feed(&bytes) {
                let _ = events.send(AppEvent::GameLine(line));
            }
            // Whatever is left unterminated is the prompt. The driver sends it
            // with no newline, which is exactly how we tell them apart.
            let _ = events.send(match ansi.partial() {
                Some(line) => AppEvent::GamePrompt(line),
                None => AppEvent::GamePrompt(ratatui::text::Line::default()),
            });
        }

        TelnetEvent::Negotiate { verb, option } => {
            // ECHO is not an ordinary option: the server toggles it repeatedly
            // around every password prompt, so it must never be de-duplicated.
            if option == OPT_ECHO {
                match verb {
                    WILL => {
                        out.extend(TelnetCodec::encode_negotiate(DO, OPT_ECHO));
                        let _ = events.send(AppEvent::Echo(true));
                    }
                    WONT => {
                        out.extend(TelnetCodec::encode_negotiate(DONT, OPT_ECHO));
                        let _ = events.send(AppEvent::Echo(false));
                    }
                    _ => {}
                }
                return;
            }

            // Everything else is answered once. Replying to a repeat is how a
            // naive client and a Q-method server talk each other into a loop.
            if !answered.insert((verb, option)) {
                return;
            }

            match (verb, option) {
                (WILL, OPT_GMCP) => {
                    out.extend(TelnetCodec::encode_negotiate(DO, OPT_GMCP));
                    // Identify, then ask for exactly the packages we render.
                    out.extend(TelnetCodec::encode_gmcp(
                        "Core.Hello",
                        Some(&format!(
                            r#"{{"client":"oxigeon-tui","version":"{}"}}"#,
                            env!("CARGO_PKG_VERSION")
                        )),
                    ));
                    out.extend(TelnetCodec::encode_gmcp(
                        "Core.Supports.Set",
                        Some(SUPPORTS),
                    ));
                }

                // MCCP2 is negotiated by the driver but never performed —
                // `flate2` appears nowhere in `src/` and `mccp2_active` is never
                // set. Accepting it would agree to a compression that never
                // starts, and every byte after would be garbage.
                (WILL, OPT_MCCP2) | (WILL, OPT_MCCP3) => {
                    out.extend(TelnetCodec::encode_negotiate(DONT, option));
                }

                (DO, OPT_NAWS) => {
                    out.extend(TelnetCodec::encode_negotiate(WILL, OPT_NAWS));
                    out.extend(naws(size.0, size.1));
                }
                (DO, OPT_TTYPE) => out.extend(TelnetCodec::encode_negotiate(WILL, OPT_TTYPE)),
                (WILL, OPT_SGA) => out.extend(TelnetCodec::encode_negotiate(DO, OPT_SGA)),
                (DO, OPT_SGA) => out.extend(TelnetCodec::encode_negotiate(WILL, OPT_SGA)),

                // Refuse anything we have not implemented, rather than leaving
                // it unanswered — an unanswered DO stalls some servers.
                (WILL, opt) => out.extend(TelnetCodec::encode_negotiate(DONT, opt)),
                (DO, opt) => out.extend(TelnetCodec::encode_negotiate(WONT, opt)),
                _ => {}
            }
        }

        TelnetEvent::Subnegotiation { option, data } => match option {
            OPT_GMCP => {
                if let Some((package, json)) = TelnetCodec::parse_gmcp(&data) {
                    let _ = events.send(AppEvent::Gmcp {
                        package,
                        json: json.unwrap_or_else(|| "null".into()),
                    });
                }
            }
            OPT_TTYPE if data.first() == Some(&TTYPE_SEND) => {
                let mut payload = vec![TTYPE_IS];
                payload.extend_from_slice(TERMINAL_TYPE.as_bytes());
                out.extend(TelnetCodec::encode_subnegotiation(OPT_TTYPE, &payload));
            }
            _ => {}
        },

        // NOP, AYT and friends. Nothing here needs a response the driver acts on.
        TelnetEvent::Command(_) => {}
    }
}

/// NAWS payload is two big-endian u16s: width then height.
fn naws(w: u16, h: u16) -> Vec<u8> {
    let payload = [(w >> 8) as u8, w as u8, (h >> 8) as u8, h as u8];
    TelnetCodec::encode_subnegotiation(OPT_NAWS, &payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn drive(input: &[u8]) -> (Vec<u8>, Vec<AppEvent>) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = TelnetParser::new();
        let mut ansi = AnsiDecoder::new();
        let mut answered = HashSet::new();
        let mut out = Vec::new();
        for event in parser.feed_bytes(input) {
            handle(event, &tx, &mut ansi, &mut answered, &mut out, (80, 24));
        }
        drop(tx);
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        (out, events)
    }

    #[test]
    fn gmcp_offer_is_accepted_and_answered_with_hello_and_supports() {
        let (out, _) = drive(&[IAC, WILL, OPT_GMCP]);
        assert!(out.starts_with(&[IAC, DO, OPT_GMCP]));
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("Core.Hello"));
        assert!(text.contains("Core.Supports.Set"));
        assert!(text.contains("Char 1"));
    }

    #[test]
    fn mccp2_is_refused_because_the_driver_never_actually_compresses() {
        let (out, _) = drive(&[IAC, WILL, OPT_MCCP2]);
        assert_eq!(out, vec![IAC, DONT, OPT_MCCP2]);
    }

    #[test]
    fn echo_toggles_masking_in_both_directions_and_is_never_deduplicated() {
        // The driver toggles ECHO around every password prompt, so the second
        // WILL must still mask. De-duplicating it would silently expose a
        // password on a re-login.
        let (out, events) = drive(&[
            IAC, WILL, OPT_ECHO, IAC, WONT, OPT_ECHO, IAC, WILL, OPT_ECHO,
        ]);
        assert_eq!(
            out,
            vec![IAC, DO, OPT_ECHO, IAC, DONT, OPT_ECHO, IAC, DO, OPT_ECHO]
        );
        let masking: Vec<bool> = events
            .iter()
            .filter_map(|e| match e {
                AppEvent::Echo(on) => Some(*on),
                _ => None,
            })
            .collect();
        assert_eq!(masking, vec![true, false, true]);
    }

    #[test]
    fn naws_is_offered_with_the_current_size() {
        let (out, _) = drive(&[IAC, DO, OPT_NAWS]);
        assert!(out.starts_with(&[IAC, WILL, OPT_NAWS]));
        // 80x24 big-endian, wrapped in a subnegotiation.
        assert!(out.ends_with(&[IAC, SB, OPT_NAWS, 0, 80, 0, 24, IAC, SE]));
    }

    #[test]
    fn terminal_type_is_reported_when_asked() {
        let (out, _) = drive(&[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
        assert_eq!(out[..3], [IAC, SB, OPT_TTYPE]);
        assert_eq!(out[3], TTYPE_IS);
        assert!(String::from_utf8_lossy(&out).contains(TERMINAL_TYPE));
    }

    #[test]
    fn an_unknown_option_is_refused_rather_than_ignored() {
        let (out, _) = drive(&[IAC, WILL, 99, IAC, DO, 98]);
        assert_eq!(out, vec![IAC, DONT, 99, IAC, WONT, 98]);
    }

    #[test]
    fn a_repeated_offer_is_answered_once() {
        let (out, _) = drive(&[IAC, WILL, OPT_SGA, IAC, WILL, OPT_SGA]);
        assert_eq!(out, vec![IAC, DO, OPT_SGA]);
    }

    #[test]
    fn game_text_and_gmcp_interleaved_in_one_read_both_arrive() {
        let mut input = b"You see a well.\r\n".to_vec();
        input.extend(TelnetCodec::encode_gmcp(
            "Char.Vitals",
            Some(r#"{"hp":42,"maxhp":50}"#),
        ));
        input.extend_from_slice(b"42h> ");
        let (_, events) = drive(&input);

        assert!(events.iter().any(|e| matches!(
            e, AppEvent::GameLine(l) if l.spans.iter().any(|s| s.content.contains("well"))
        )));
        assert!(events.iter().any(|e| matches!(
            e, AppEvent::Gmcp { package, json } if package == "Char.Vitals" && json.contains("42")
        )));
        assert!(events.iter().any(|e| matches!(
            e, AppEvent::GamePrompt(l) if l.spans.iter().any(|s| s.content.contains("42h>"))
        )));
    }
}
