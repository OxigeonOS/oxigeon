//! The JSON envelope a WebSocket client speaks.
//!
//! Telnet carries an undifferentiated byte stream and signals everything else
//! out of band, in IAC sequences the text is escaped against. A frame protocol
//! has no such channel and needs none: every message says what it is.
//!
//! ```json
//! → {"type":"text","text":"You are in a forest clearing."}
//! → {"type":"prompt","text":"HP:40/40 > "}
//! → {"type":"gmcp","package":"Char.Vitals","data":{"hp":40}}
//! → {"type":"echo","masked":true}
//! ← {"type":"input","text":"look"}
//! ← {"type":"hello","width":100,"height":40,"gmcp":true,"terminal":"web"}
//! ```
//!
//! The tag strings are a contract with clients that do not live in this
//! repository. The unit tests below assert them literally rather than deriving
//! them, so a rename that looks harmless here fails loudly.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::core::SessionOutput;

// ─── client → server ─────────────────────────────────────────────────────────

/// A frame from the client.
///
/// An unrecognised `type` is answered with [`ServerFrame::Error`] and the
/// connection stays open. A running server outlives several versions of a
/// browser client, and closing on an unknown frame makes every deploy hostile.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    /// One line of input — what `on_input` receives, with no line terminator.
    /// Embedded newlines are split, so a paste is several commands rather than
    /// one command containing newlines that no mudlib parser expects.
    Input { text: String },

    /// Capability announcement. Optional, and repeatable: a browser reports a
    /// window resize by sending it again, which is this transport's NAWS.
    Hello {
        #[serde(default)]
        width: Option<u16>,
        #[serde(default)]
        height: Option<u16>,
        /// Defaults true. A client that did not want GMCP would not have
        /// connected to a JSON envelope.
        #[serde(default = "default_true")]
        gmcp: bool,
        #[serde(default)]
        terminal: Option<String>,
        /// How the client wants colour. Defaults to `raw`, so a client written
        /// before this field existed keeps getting exactly what it got.
        #[serde(default)]
        ansi: AnsiMode,
    },

    /// Inbound GMCP, the same shape as the outbound frame. Already a JSON
    /// value, so unlike the telnet path there is nothing to parse.
    Gmcp {
        package: String,
        #[serde(default)]
        data: JsonValue,
    },

    /// Application-level keepalive, answered with [`ServerFrame::Pong`].
    /// Optional — the server also sends RFC 6455 pings, which a browser answers
    /// inside the stack without the page ever seeing them. This exists for
    /// clients that want to measure the round trip themselves.
    Ping,
}

fn default_true() -> bool {
    true
}

// ─── colour ──────────────────────────────────────────────────────────────────

/// What a client wants done with the escape codes the mudlib emits.
///
/// The driver does not decide this. `Player:_process_output` already strips
/// colour for a player who has turned it off, which is a *game* preference the
/// mudlib owns; this is a *client capability* — whether the thing on the far
/// end can render an escape code at all. A terminal can, a `<div>` cannot, and
/// neither of them is a statement about what the player prefers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnsiMode {
    /// Escape codes pass through untouched. The default, and what every client
    /// written before `spans` existed already gets.
    #[default]
    Raw,
    /// Parsed into structured spans. For a browser, which would otherwise need
    /// its own SGR state machine to render a `<span>`.
    Spans,
    /// Stripped. For a client that wants neither — a log tail, a bot.
    None,
}

impl AnsiMode {
    /// Render one message, returning whichever of `text`/`spans` this mode
    /// populates. Exactly one is `Some`.
    fn render(self, s: &str) -> (Option<String>, Option<Vec<Span>>) {
        match self {
            AnsiMode::Raw => (Some(s.to_string()), None),
            AnsiMode::None => (Some(strip_ansi(s)), None),
            AnsiMode::Spans => (None, Some(to_spans(s))),
        }
    }
}

/// A run of text sharing one style.
///
/// Colours are **xterm-256 palette indices**, always — the 16 basic colours are
/// 0-15 in that palette, so one integer covers every case the mudlib can emit
/// and a client needs one lookup table rather than three. A 24-bit `38;2;r;g;b`
/// sequence, which the mudlib does not produce but a hand-written `send()`
/// could, is folded to the nearest palette entry rather than widening the type.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Span {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fg: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg: Option<u8>,
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub dim: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub blink: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub inverse: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub strike: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The style carried across a sequence of spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Style {
    fg: Option<u8>,
    bg: Option<u8>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    inverse: bool,
    strike: bool,
}

impl Style {
    fn span(self, text: String) -> Span {
        Span {
            text,
            fg: self.fg,
            bg: self.bg,
            bold: self.bold,
            dim: self.dim,
            italic: self.italic,
            underline: self.underline,
            blink: self.blink,
            inverse: self.inverse,
            strike: self.strike,
        }
    }
}

/// One piece of a scanned string: a style change, or text to render.
enum Piece<'a> {
    Sgr(&'a str),
    Text(&'a str),
}

/// Walk a string, emitting each SGR sequence and each run of ordinary text.
///
/// One scanner behind both `to_spans` and `strip_ansi`, so the two cannot
/// disagree about where a sequence ends — which is the failure mode where
/// stripping leaves a stray `m` behind and spans do not, or the reverse.
///
/// Only SGR (`ESC [ … m`) is interpreted. Any other CSI sequence — a cursor
/// move, an erase — is recognised and **dropped**: a browser cannot act on one,
/// and letting its parameter bytes fall through as text is how `[2J` ends up
/// printed in the scrollback. A bare `ESC` that begins nothing is dropped too.
fn scan(s: &str, mut emit: impl FnMut(Piece<'_>)) {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut run_start = 0;

    while i < bytes.len() {
        if bytes[i] != 0x1b {
            i += 1;
            continue;
        }
        if run_start < i {
            emit(Piece::Text(&s[run_start..i]));
        }
        // ESC [ … final-byte
        if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let params_start = i + 2;
            let mut j = params_start;
            // Parameter and intermediate bytes, then one final byte 0x40-0x7E.
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            if j < bytes.len() {
                if bytes[j] == b'm' {
                    emit(Piece::Sgr(&s[params_start..j]));
                }
                i = j + 1;
            } else {
                // Truncated sequence at end of input: nothing left to emit.
                i = bytes.len();
            }
        } else {
            // A lone ESC, or ESC followed by something that is not a CSI.
            i += 1;
        }
        run_start = i;
    }

    if run_start < bytes.len() {
        emit(Piece::Text(&s[run_start..]));
    }
}

/// Split text into styled runs.
pub fn to_spans(s: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut style = Style::default();
    let mut buf = String::new();

    scan(s, |piece| match piece {
        Piece::Sgr(params) => {
            // Flush before the style changes, not after: the text already
            // buffered belongs to the *old* style.
            if !buf.is_empty() {
                spans.push(style.span(std::mem::take(&mut buf)));
            }
            apply_sgr(&mut style, params);
        }
        Piece::Text(t) => buf.push_str(t),
    });

    if !buf.is_empty() {
        spans.push(style.span(buf));
    }
    spans
}

/// The same text with every escape sequence removed.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    scan(s, |piece| {
        if let Piece::Text(t) = piece {
            out.push_str(t);
        }
    });
    out
}

/// Apply one SGR parameter string to a running style.
fn apply_sgr(style: &mut Style, params: &str) {
    // `ESC[m` is `ESC[0m`.
    if params.is_empty() {
        *style = Style::default();
        return;
    }

    let parts: Vec<&str> = params.split(';').collect();
    let mut i = 0;
    while i < parts.len() {
        // An empty parameter is zero, per ECMA-48.
        let n: u16 = if parts[i].is_empty() {
            0
        } else {
            match parts[i].parse() {
                Ok(n) => n,
                // Unparseable: skip it rather than abandoning the rest of the
                // sequence, which would leave the style half-applied.
                Err(_) => {
                    i += 1;
                    continue;
                }
            }
        };

        match n {
            0 => *style = Style::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            5 | 6 => style.blink = true,
            7 => style.inverse = true,
            9 => style.strike = true,
            21 | 22 => {
                style.bold = false;
                style.dim = false;
            }
            23 => style.italic = false,
            24 => style.underline = false,
            25 => style.blink = false,
            27 => style.inverse = false,
            29 => style.strike = false,
            30..=37 => style.fg = Some((n - 30) as u8),
            39 => style.fg = None,
            40..=47 => style.bg = Some((n - 40) as u8),
            49 => style.bg = None,
            90..=97 => style.fg = Some((n - 90 + 8) as u8),
            100..=107 => style.bg = Some((n - 100 + 8) as u8),
            38 | 48 => {
                let target_fg = n == 38;
                match parts.get(i + 1).and_then(|p| p.parse::<u16>().ok()) {
                    // 5;N — palette index, which is what the mudlib emits.
                    Some(5) => {
                        if let Some(v) = parts.get(i + 2).and_then(|p| p.parse::<u16>().ok()) {
                            let c = v.min(255) as u8;
                            if target_fg { style.fg = Some(c) } else { style.bg = Some(c) }
                        }
                        i += 3;
                        continue;
                    }
                    // 2;R;G;B — folded to the palette. See `Span`.
                    Some(2) => {
                        let get = |k: usize| {
                            parts.get(i + k).and_then(|p| p.parse::<u16>().ok()).unwrap_or(0) as u8
                        };
                        let c = nearest_xterm(get(2), get(3), get(4));
                        if target_fg { style.fg = Some(c) } else { style.bg = Some(c) }
                        i += 5;
                        continue;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// Fold a 24-bit colour onto the xterm-256 palette.
///
/// The 6×6×6 colour cube, or the 24-step grey ramp when the channels are close
/// enough to call it grey — the ramp is much finer than the cube's four
/// interior grey points, so text that was meant to be grey stays grey.
fn nearest_xterm(r: u8, g: u8, b: u8) -> u8 {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max - min < 10 {
        let level = ((max as u16 * 23) / 255) as u8;
        return 232 + level;
    }
    let q = |v: u8| -> u16 {
        // The cube's levels are 0, 95, 135, 175, 215, 255 — not evenly spaced.
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, &l)| (l as i16 - v as i16).abs())
            .map(|(i, _)| i as u16)
            .unwrap_or(0)
    };
    (16 + 36 * q(r) + 6 * q(g) + q(b)) as u8
}

// ─── server → client ─────────────────────────────────────────────────────────

/// A frame to the client. One per `SessionOutput`, which is one per `send()`
/// from Lua.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    /// A block of game text: `\n`-separated, no trailing terminator. See
    /// [`normalize`] for the line endings.
    ///
    /// Exactly one of `text` and `spans` is present, decided by the `ansi` mode
    /// the client asked for in its `hello`. `spans` mode omits `text` rather
    /// than sending both: this is the busiest frame in the protocol and
    /// duplicating its content would double it.
    Text {
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        spans: Option<Vec<Span>>,
    },

    /// Text with no terminator, belonging on the input line rather than in the
    /// scrollback. Same `text`/`spans` rule as above.
    Prompt {
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        spans: Option<Vec<Span>>,
    },

    Gmcp { package: String, data: JsonValue },

    /// `masked: true` — hide what the player types, a password is being asked
    /// for. `false` — show it again.
    ///
    /// The polarity is inverted relative to the efun names that produce it and
    /// this is the single most likely thing for a client author to get
    /// backwards. `start_echo` means *the server* will echo, so the client must
    /// stop; that is why this field is `masked` and not `echo`. Getting it
    /// wrong puts a password into a browser's DOM.
    Echo { masked: bool },

    /// The server is ending the session. Sent before the close frame so a
    /// client can tell an intentional goodbye from a dropped socket.
    Bye {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// A malformed or unrecognised client frame. Advisory: the session lives.
    Error { message: String },

    Pong,
}

impl ServerFrame {
    /// Turn one message from Lua into one frame, in the client's chosen
    /// rendering.
    ///
    /// A free function rather than `From`, because the answer depends on what
    /// the client asked for in its `hello` and `From` has nowhere to put that.
    /// Exhaustive on purpose: a seventh `SessionOutput` variant should be a
    /// compile error here, not a message this transport silently drops.
    pub fn from_output(out: SessionOutput, ansi: AnsiMode) -> Self {
        match out {
            SessionOutput::Text(text) => {
                let (text, spans) = ansi.render(&normalize(&text));
                ServerFrame::Text { text, spans }
            }

            // `Raw` is a prompt. Not a heuristic — `SessionOutput::Raw` is
            // constructed in exactly one place in the crate, inside the
            // `send_prompt` efun, and the testkit already reads it as the
            // definition of a prompt. The variant is named for its telnet
            // *implementation* ("raw bytes, no CRLF appended") rather than its
            // meaning, which is why this needs saying twice.
            //
            // `send_prompt` builds the bytes with `String::into_bytes`, so they
            // are valid UTF-8 by construction. Lossy anyway: it is total, free
            // on the happy path, and cannot panic in a connection task where a
            // panic reads to the player as a mysterious disconnect.
            //
            // Not normalized: a prompt exists precisely to leave the cursor on
            // the line. It is still split into spans, because a prompt carries
            // as much colour as anything else the mudlib writes.
            SessionOutput::Raw(bytes) => {
                let (text, spans) = ansi.render(&String::from_utf8_lossy(&bytes));
                ServerFrame::Prompt { text, spans }
            }

            // A rich line becomes an ordinary text or prompt frame, with the
            // action dropped — the same degradation a plain telnet client gets,
            // and the reason a rich line is safe to send to anybody.
            //
            // No new frame type, deliberately. A `ServerFrame::Rich` would mean
            // every client needed a new branch before it could render game
            // text at all, and `debug-client/` and any third-party client would
            // silently drop it. Carrying the action as extra optional `Span`
            // fields — so a browser could render a button — is the natural next
            // step and is wire-compatible with this one, because every existing
            // field would be unchanged.
            //
            // Never `to_mxp`: `AnsiMode::Raw` passes a non-SGR CSI through
            // untouched, so an `ESC[1z` composed here would land in a browser's
            // DOM. Choosing the rendering at the transport is what prevents it.
            SessionOutput::Rich(line) => {
                let rendered = crate::core::render::to_text(&line);
                if line.newline {
                    let (text, spans) = ansi.render(&normalize(&rendered));
                    ServerFrame::Text { text, spans }
                } else {
                    let (text, spans) = ansi.render(&rendered);
                    ServerFrame::Prompt { text, spans }
                }
            }

            // The value nests. The telnet path has to `data.to_string()`
            // because GMCP rides a subnegotiation that carries bytes; here the
            // envelope is already JSON and re-encoding it would hand every
            // client a string to parse a second time.
            SessionOutput::Gmcp { package, data } => ServerFrame::Gmcp { package, data },

            SessionOutput::StartEcho => ServerFrame::Echo { masked: true },
            SessionOutput::StopEcho => ServerFrame::Echo { masked: false },

            SessionOutput::Disconnect => ServerFrame::Bye { reason: None },
        }
    }

    /// A plain-text frame, for the transport's own messages — which carry no
    /// colour and should not depend on what the client negotiated.
    pub fn plain(text: impl Into<String>) -> Self {
        ServerFrame::Text { text: Some(text.into()), spans: None }
    }
}

// ─── line endings ────────────────────────────────────────────────────────────

/// Strip the line terminator the mudlib appends, and make the interior
/// consistent.
///
/// `Player:_process_output` ends every message with `send(sid, text .. "\r\n")`
/// and `strings.wrap` joins with `\r\n`. On telnet that is correct, and
/// `TelnetCodec::encode_text` even rewrites bare `\n` into CRLF on the way out
/// — the driver is already in the business of per-transport line endings. Here
/// a `\r` is at best invisible and at worst a stray glyph, so it goes.
///
/// Doing it once, here, is what keeps it out of every client. The alternative
/// is that the first client to forget ships a UI with a phantom blank line
/// after every message.
///
/// Exactly one trailing terminator is removed, never all of them: a deliberate
/// blank final line is content.
pub fn normalize(s: &str) -> String {
    let body = s
        .strip_suffix("\r\n")
        .or_else(|| s.strip_suffix('\n'))
        .or_else(|| s.strip_suffix('\r'))
        .unwrap_or(s);
    body.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── client frames ────────────────────────────────────────────────

    #[test]
    fn input_parses() {
        let f: ClientFrame = serde_json::from_value(json!({"type":"input","text":"look"})).unwrap();
        match f {
            ClientFrame::Input { text } => assert_eq!(text, "look"),
            other => panic!("expected input, got {other:?}"),
        }
    }

    #[test]
    fn a_hello_with_every_optional_absent_still_means_gmcp() {
        // The default matters: a client that says hello only to set its width
        // must not thereby turn GMCP off.
        let f: ClientFrame = serde_json::from_value(json!({"type":"hello"})).unwrap();
        match f {
            ClientFrame::Hello { width, height, gmcp, terminal, ansi } => {
                assert_eq!(width, None);
                assert_eq!(height, None);
                assert_eq!(terminal, None);
                assert!(gmcp, "gmcp must default on");
                assert_eq!(
                    ansi,
                    AnsiMode::Raw,
                    "a client that never mentions ansi must keep getting escape codes"
                );
            }
            other => panic!("expected hello, got {other:?}"),
        }
    }

    #[test]
    fn a_hello_carries_its_fields() {
        let f: ClientFrame = serde_json::from_value(
            json!({"type":"hello","width":132,"height":50,"gmcp":false,
                   "terminal":"web","ansi":"spans"}),
        )
        .unwrap();
        match f {
            ClientFrame::Hello { width, height, gmcp, terminal, ansi } => {
                assert_eq!(width, Some(132));
                assert_eq!(height, Some(50));
                assert_eq!(terminal.as_deref(), Some("web"));
                assert!(!gmcp);
                assert_eq!(ansi, AnsiMode::Spans);
            }
            other => panic!("expected hello, got {other:?}"),
        }
    }

    #[test]
    fn inbound_gmcp_parses_with_and_without_data() {
        let f: ClientFrame =
            serde_json::from_value(json!({"type":"gmcp","package":"Core.Ping"})).unwrap();
        match f {
            ClientFrame::Gmcp { package, data } => {
                assert_eq!(package, "Core.Ping");
                assert!(data.is_null());
            }
            other => panic!("expected gmcp, got {other:?}"),
        }

        let f: ClientFrame = serde_json::from_value(
            json!({"type":"gmcp","package":"Core.Supports.Set","data":["Char 1"]}),
        )
        .unwrap();
        match f {
            ClientFrame::Gmcp { data, .. } => assert!(data.is_array()),
            other => panic!("expected gmcp, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_type_is_an_error_not_a_panic() {
        assert!(serde_json::from_value::<ClientFrame>(json!({"type":"nope"})).is_err());
        assert!(serde_json::from_str::<ClientFrame>("not json").is_err());
        // A known type missing its required field is equally a decode failure,
        // not a default-filled frame.
        assert!(serde_json::from_value::<ClientFrame>(json!({"type":"input"})).is_err());
    }

    // ── server frames ────────────────────────────────────────────────

    #[test]
    fn server_frames_carry_their_tag() {
        // Asserted literally. These strings are the contract with a client that
        // is not in this repository, so deriving them from the enum would let a
        // rename pass silently and break every client at once.
        let cases = [
            (ServerFrame::plain("hi"), json!({"type":"text","text":"hi"})),
            (
                ServerFrame::Prompt { text: Some("> ".into()), spans: None },
                json!({"type":"prompt","text":"> "}),
            ),
            (
                ServerFrame::Text {
                    text: None,
                    spans: Some(vec![Span { text: "hi".into(), fg: Some(1), bold: true, ..Default::default() }]),
                },
                json!({"type":"text","spans":[{"text":"hi","fg":1,"bold":true}]}),
            ),
            (
                ServerFrame::Gmcp { package: "Char.Vitals".into(), data: json!({"hp":40}) },
                json!({"type":"gmcp","package":"Char.Vitals","data":{"hp":40}}),
            ),
            (ServerFrame::Echo { masked: true }, json!({"type":"echo","masked":true})),
            (ServerFrame::Echo { masked: false }, json!({"type":"echo","masked":false})),
            (ServerFrame::Bye { reason: None }, json!({"type":"bye"})),
            (
                ServerFrame::Bye { reason: Some("kicked".into()) },
                json!({"type":"bye","reason":"kicked"}),
            ),
            (
                ServerFrame::Error { message: "bad frame".into() },
                json!({"type":"error","message":"bad frame"}),
            ),
            (ServerFrame::Pong, json!({"type":"pong"})),
        ];
        for (frame, expected) in cases {
            assert_eq!(serde_json::to_value(&frame).unwrap(), expected, "for {frame:?}");
        }
    }

    // ── the mapping ──────────────────────────────────────────────────

    #[test]
    fn every_session_output_maps() {
        // Named one by one rather than leaning on the compiler's exhaustiveness
        // check: that pins *that* each variant is handled, this pins *how*.
        let raw = AnsiMode::Raw;
        assert_eq!(
            ServerFrame::from_output(SessionOutput::Text("hello\r\n".into()), raw),
            ServerFrame::plain("hello")
        );
        assert_eq!(
            ServerFrame::from_output(SessionOutput::Raw(b"> ".to_vec()), raw),
            ServerFrame::Prompt { text: Some("> ".into()), spans: None }
        );
        assert_eq!(
            ServerFrame::from_output(SessionOutput::Gmcp {
                package: "Room.Info".into(),
                data: json!({"name":"A clearing"}),
            }, raw),
            ServerFrame::Gmcp {
                package: "Room.Info".into(),
                data: json!({"name":"A clearing"}),
            }
        );
        assert_eq!(
            ServerFrame::from_output(SessionOutput::StartEcho, raw),
            ServerFrame::Echo { masked: true },
            "start_echo means the server echoes, so the client must not — the password is hidden"
        );
        assert_eq!(
            ServerFrame::from_output(SessionOutput::StopEcho, raw),
            ServerFrame::Echo { masked: false }
        );
        assert_eq!(
            ServerFrame::from_output(SessionOutput::Disconnect, raw),
            ServerFrame::Bye { reason: None }
        );
    }

    #[test]
    fn a_prompt_keeps_its_trailing_space_and_gains_no_newline() {
        // `send_prompt` exists precisely to *not* terminate the line. If this
        // ever went through `normalize` the cursor would land in the wrong
        // place on every client.
        let f = ServerFrame::from_output(SessionOutput::Raw(b"HP:40/40 > ".to_vec()), AnsiMode::Raw);
        assert_eq!(f, ServerFrame::Prompt { text: Some("HP:40/40 > ".into()), spans: None });
    }

    #[test]
    fn gmcp_data_stays_nested() {
        let f = ServerFrame::from_output(
            SessionOutput::Gmcp {
                package: "Char.Vitals".into(),
                data: json!({"hp": 40, "maxhp": 50}),
            },
            AnsiMode::Raw,
        );
        let v = serde_json::to_value(&f).unwrap();
        assert!(v["data"].is_object(), "data must nest, not be a JSON-encoded string: {v}");
        assert_eq!(v["data"]["hp"], 40);
    }

    #[test]
    fn ansi_survives_a_round_trip() {
        // serde_json escapes ESC as . A client reading it back must get
        // the same byte, or every colour code arrives mangled.
        let text = "\u{1b}[31mred\u{1b}[0m";
        let wire = serde_json::to_string(&ServerFrame::plain(text)).unwrap();
        assert!(wire.contains("\\u001b"), "ESC should be escaped in the wire form: {wire}");
        let back: JsonValue = serde_json::from_str(&wire).unwrap();
        assert_eq!(back["text"].as_str().unwrap(), text);
    }

    // ── line endings ─────────────────────────────────────────────────

    #[test]
    fn normalize_strips_exactly_one_trailing_terminator() {
        assert_eq!(normalize("a\r\nb\r\n"), "a\nb");
        assert_eq!(normalize("a\nb\n"), "a\nb");
        assert_eq!(normalize("a"), "a");
        // A deliberate blank final line is content and survives.
        assert_eq!(normalize("a\r\n\r\n"), "a\n");
    }

    #[test]
    fn normalize_rewrites_interior_carriage_returns() {
        assert_eq!(normalize("a\r\nb\r\nc"), "a\nb\nc");
        // A bare CR the mudlib never produces, but the pager's cursor hack
        // comes close enough that it should not reach a browser as a glyph.
        assert_eq!(normalize("a\rb"), "a\nb");
    }

    // ── colour spans ─────────────────────────────────────────────────

    /// The mudlib's own vocabulary: `lib/color.lua` emits exactly these.
    const RED: &str = "\u{1b}[31m";
    const BOLD: &str = "\u{1b}[1m";
    const RESET: &str = "\u{1b}[0m";

    fn span(text: &str) -> Span {
        Span { text: text.into(), ..Default::default() }
    }

    #[test]
    fn plain_text_is_one_unstyled_span() {
        assert_eq!(to_spans("hello"), vec![span("hello")]);
    }

    #[test]
    fn a_colour_code_starts_a_new_span_and_reset_ends_it() {
        let out = to_spans(&format!("a{RED}b{RESET}c"));
        assert_eq!(
            out,
            vec![
                span("a"),
                Span { text: "b".into(), fg: Some(1), ..Default::default() },
                span("c"),
            ]
        );
    }

    #[test]
    fn styles_accumulate_until_reset() {
        // `{red}{bold}` is two sequences, and the second must not discard the
        // first — which is the whole reason a span carries a running style
        // rather than the last code seen.
        let out = to_spans(&format!("{RED}{BOLD}x"));
        assert_eq!(
            out,
            vec![Span { text: "x".into(), fg: Some(1), bold: true, ..Default::default() }]
        );
    }

    #[test]
    fn the_sixteen_basic_colours_land_on_palette_indices_0_to_15() {
        // 30-37 are 0-7 and 90-97 are 8-15 in the xterm palette, which is what
        // lets a span carry one integer instead of a colour *kind* and a value.
        let out = to_spans("\u{1b}[32ma\u{1b}[94mb\u{1b}[41mc");
        assert_eq!(out[0].fg, Some(2), "green is 2");
        assert_eq!(out[1].fg, Some(12), "bright blue is 12");
        assert_eq!(out[2].fg, Some(12), "an unrelated bg must not clear the fg");
        assert_eq!(out[2].bg, Some(1), "red background is 1");
    }

    #[test]
    fn a_256_colour_sequence_keeps_its_index() {
        // `{orange}` in the mudlib is 208.
        let out = to_spans("\u{1b}[38;5;208mo");
        assert_eq!(out, vec![Span { text: "o".into(), fg: Some(208), ..Default::default() }]);
        let out = to_spans("\u{1b}[48;5;17mb");
        assert_eq!(out, vec![Span { text: "b".into(), bg: Some(17), ..Default::default() }]);
    }

    #[test]
    fn a_truecolour_sequence_folds_onto_the_palette() {
        // The mudlib never emits this, but a hand-written `send` could, and
        // dropping it would silently lose the colour.
        let out = to_spans("\u{1b}[38;2;255;0;0mr");
        assert_eq!(out[0].fg, Some(196), "pure red is cube entry 196");
        // Near-grey takes the grey ramp, which is far finer than the cube.
        let out = to_spans("\u{1b}[38;2;128;128;128mg");
        assert!((232..=255).contains(&out[0].fg.unwrap()), "grey should use the ramp");
    }

    #[test]
    fn style_off_codes_clear_only_their_own_style() {
        let out = to_spans("\u{1b}[1;4ma\u{1b}[24mb");
        assert_eq!(out[0].bold, true);
        assert_eq!(out[0].underline, true);
        assert_eq!(out[1].bold, true, "24 turns off underline, not bold");
        assert_eq!(out[1].underline, false);
    }

    #[test]
    fn a_bare_escape_bracket_m_is_a_reset() {
        // ECMA-48: an omitted parameter is zero.
        let out = to_spans(&format!("{RED}a\u{1b}[mb"));
        assert_eq!(out[1], span("b"));
    }

    #[test]
    fn a_non_sgr_sequence_is_dropped_not_printed() {
        // A cursor move or an erase cannot be rendered in a browser, and
        // letting its bytes through is how `[2J` ends up in the scrollback.
        assert_eq!(to_spans("a\u{1b}[2Jb"), vec![span("ab")]);
        assert_eq!(strip_ansi("a\u{1b}[2Jb"), "ab");
    }

    #[test]
    fn a_truncated_sequence_at_the_end_does_not_panic_or_leak() {
        assert_eq!(to_spans("abc\u{1b}[3"), vec![span("abc")]);
        assert_eq!(to_spans("abc\u{1b}"), vec![span("abc")]);
        assert_eq!(strip_ansi("abc\u{1b}[3"), "abc");
    }

    #[test]
    fn an_empty_run_produces_no_span() {
        // Two codes in a row, and codes at either end, must not emit empty
        // spans — a client rendering one gets a stray empty element per colour
        // change, which is most of them.
        let out = to_spans(&format!("{RED}{BOLD}x{RESET}"));
        assert_eq!(out.len(), 1);
        assert!(to_spans(RESET).is_empty());
        assert!(to_spans("").is_empty());
    }

    #[test]
    fn strip_and_spans_agree_about_the_text() {
        let s = format!("{RED}a{BOLD}b{RESET}c\u{1b}[38;5;208md");
        let joined: String = to_spans(&s).iter().map(|sp| sp.text.as_str()).collect();
        assert_eq!(joined, strip_ansi(&s));
        assert_eq!(joined, "abcd");
    }

    #[test]
    fn the_ansi_mode_decides_which_field_is_populated() {
        let coloured = format!("{RED}hi{RESET}");
        let out = SessionOutput::Text(format!("{coloured}\r\n"));

        match ServerFrame::from_output(SessionOutput::Text(format!("{coloured}\r\n")), AnsiMode::Raw) {
            ServerFrame::Text { text, spans } => {
                assert!(spans.is_none());
                assert!(text.unwrap().contains('\u{1b}'), "raw keeps the escape codes");
            }
            other => panic!("expected text, got {other:?}"),
        }

        match ServerFrame::from_output(SessionOutput::Text(format!("{coloured}\r\n")), AnsiMode::None) {
            ServerFrame::Text { text, spans } => {
                assert!(spans.is_none());
                assert_eq!(text.unwrap(), "hi");
            }
            other => panic!("expected text, got {other:?}"),
        }

        match ServerFrame::from_output(out, AnsiMode::Spans) {
            ServerFrame::Text { text, spans } => {
                assert!(text.is_none(), "spans mode must not also send text");
                assert_eq!(
                    spans.unwrap(),
                    vec![Span { text: "hi".into(), fg: Some(1), ..Default::default() }]
                );
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_prompt_carries_spans_too_and_is_still_not_normalized() {
        let f = ServerFrame::from_output(
            SessionOutput::Raw(format!("{RED}HP:40/40 > ").into_bytes()),
            AnsiMode::Spans,
        );
        match f {
            ServerFrame::Prompt { text, spans } => {
                assert!(text.is_none());
                let spans = spans.unwrap();
                assert_eq!(spans[0].fg, Some(1));
                assert_eq!(spans[0].text, "HP:40/40 > ", "the trailing space survives");
            }
            other => panic!("expected prompt, got {other:?}"),
        }
    }

    #[test]
    fn normalize_leaves_an_empty_string_alone() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("\r\n"), "");
    }
}
