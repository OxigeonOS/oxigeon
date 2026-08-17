//! MXP — the MUD eXtension Protocol, telnet option 91.
//!
//! MXP is a markup language for the game text stream: `<SEND href="buy bread">`
//! makes a word clickable, `<VAR hp>40</VAR>` sets a client-side variable,
//! `ESC[10z` tells an automapper that this line is a room name. The
//! specification is Zugg Software's, version 1.0 of 12-Mar-2003, and it is
//! public domain.
//!
//! ## The security model, which is the whole reason this file is careful
//!
//! MXP marks each *line* as open, secure or locked, with an escape sequence
//! that looks like an ANSI one: `ESC [ <n> z`. On an open line only the
//! presentational tags parse; on a secure line everything does, including
//! `<SEND>`. The spec's own warning is worth quoting because it is the design
//! constraint:
//!
//! > Be very careful when sending Secure lines from the MUD. Be absolutely
//! > sure that MUD players cannot control the output of a secure line. If a MUD
//! > player is able to send a secure MXP command, he will be able to cause
//! > great damage to other MUD players using MXP.
//!
//! A MUD is a machine for taking one player's text and showing it to another,
//! so "players cannot control the output" is not something a game gets right by
//! being careful. This driver gets it by construction, in two steps:
//!
//! 1. **The default line mode is LOCKED.** Immediately after negotiation the
//!    driver sends `ESC[7z`, which makes LOCKED the default every line reverts
//!    to. A locked line is not parsed at all, so a mob named
//!    `<send href="quit">bow</send>` is a silly name and not a command the
//!    player's client runs for them. Ordinary `send()` output is therefore
//!    passed through byte for byte, exactly as it was before MXP existed.
//! 2. **Line-mode sequences are stripped from game text** — see
//!    [`strip_line_modes`]. Mode tags are honoured in *every* mode, including
//!    locked; that is how a client gets back out of locked mode. So a player
//!    who types `ESC[1z<send href="quit">free gold</send>` into `say` would
//!    otherwise promote their own line to secure on everybody else's screen.
//!    That strip is what closes it, and it is the load-bearing part of this
//!    module.
//!
//! Markup reaches the wire through exactly one door: [`crate::core::render`],
//! which builds a line from a structured tree and is the only code in the
//! driver permitted to emit a `<`. Every piece of caller-supplied text in that
//! tree goes through [`escape`] first. There is no way to hand this module a
//! string of markup and have it trusted, because there is no call site that
//! could then be forgotten.
//!
//! ## What is not here
//!
//! `<!ELEMENT>`, `<!ENTITY>`, `<!ATTLIST>` and `<!TAG>` define client-side
//! macros. They are a second data channel that would compete with GMCP, which
//! this driver already speaks and which carries structured values better. Line
//! tags (10/11/12, 19, 20-99) cover the styling case at a fraction of the cost
//! and are supported. `<FRAME>`, `<DEST>`, `<IMAGE>`, `<SOUND>` and `<MUSIC>`
//! are content decisions belonging to a game, and `<RELOCATE>` — which tells a
//! client to connect somewhere else — has no legitimate use from a driver.

use std::borrow::Cow;

use super::codec::TelnetCodec;
use super::constants::*;
use crate::core::session::ClientCapabilities;

// ─── line security modes ─────────────────────────────────────────────────────

/// Only the presentational tags parse: `<B> <I> <U> <S> <C> <H> <FONT>`.
pub const MODE_OPEN: u8 = 0;
/// Every tag parses, `<SEND>` included. Reverts at the newline.
pub const MODE_SECURE: u8 = 1;
/// Nothing parses. Verbatim text.
pub const MODE_LOCKED: u8 = 2;
/// Close every open tag, reset colour, back to open.
pub const MODE_RESET: u8 = 3;
/// Secure for exactly the next tag, which must follow immediately.
pub const MODE_TEMP_SECURE: u8 = 4;
/// Open becomes the new default.
pub const MODE_LOCK_OPEN: u8 = 5;
/// Secure becomes the new default. **Never sent by this driver** — see
/// [`MxpState::enable`].
pub const MODE_LOCK_SECURE: u8 = 6;
/// Locked becomes the new default. Sent once, at the start of every session.
pub const MODE_LOCK_LOCKED: u8 = 7;

/// This line is a room name (automapper).
pub const MODE_ROOM_NAME: u8 = 10;
/// This line is a room description (automapper).
pub const MODE_ROOM_DESC: u8 = 11;
/// This line is a room's exits (automapper).
pub const MODE_ROOM_EXITS: u8 = 12;
/// Welcome text, suppressed by a client that arrived via `<RELOCATE>`.
pub const MODE_WELCOME: u8 = 19;

/// The lowest and highest user-defined line tags. A game assigns meaning to
/// these; a client lets the player recolour, gag or redirect them.
pub const USER_TAG_MIN: u8 = 20;
pub const USER_TAG_MAX: u8 = 99;

/// How a line will be parsed by the client.
///
/// Only three of the eight modes are states a line can be *in*; the rest are
/// instructions (reset, lock, temp-secure). Modelling only the states keeps the
/// question "what mode is the far end in" answerable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineMode {
    /// Presentational tags only.
    Open,
    /// Everything. Only ever driver-authored content.
    Secure,
    /// Nothing. The default this driver locks the stream to.
    #[default]
    Locked,
}

impl LineMode {
    /// The `ESC[<n>z` code that selects this mode for one line.
    pub fn code(self) -> u8 {
        match self {
            LineMode::Open => MODE_OPEN,
            LineMode::Secure => MODE_SECURE,
            LineMode::Locked => MODE_LOCKED,
        }
    }
}

/// `ESC [ <n> z`.
///
/// The number is decimal **text**, as in ANSI — `line_tag(19)` is six bytes
/// with two digits in the middle, not five with a `19` byte. Writing the code
/// as a raw byte would send a control character and set no mode at all.
pub fn line_tag(n: u8) -> String {
    format!("\x1b[{n}z")
}

// ─── connection state ────────────────────────────────────────────────────────

/// Everything MXP needs to know about one connection.
///
/// Lives on [`super::TelnetConnection`] beside `negotiator` and `capabilities`,
/// and is driven by `relay.rs` exactly as those two are.
#[derive(Debug, Clone)]
pub struct MxpState {
    /// Whether this listener offers MXP at all. Configuration, set once.
    offered: bool,
    /// The client accepted and the start sequence went out.
    enabled: bool,
}

impl MxpState {
    pub fn new(offered: bool) -> Self {
        MxpState { offered, enabled: false }
    }

    /// Whether to include MXP in the opening negotiation burst.
    pub fn offered(&self) -> bool {
        self.offered
    }

    /// Whether the client is parsing markup right now.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The client accepted. Returns every byte that turns MXP on.
    ///
    /// **Empty if MXP is already on**, which is what makes a repeated
    /// `IAC DO 91` harmless. The GMCP arm beside this one re-pushes
    /// `Core.Hello` on every repeat; doing the same here would drop a second
    /// `ESC[7z` and two more handshake queries into the middle of a page the
    /// player is reading.
    ///
    /// The sequence, in order:
    ///
    /// * `IAC SB 91 IAC SE` — the client's parser starts here. The spec puts
    ///   this as a "should"; some clients begin on the `DO` alone and ignore
    ///   the empty subnegotiation, others wait for it. Five bytes either way.
    /// * `ESC[7z` — LOCK LOCKED. Every line the mudlib writes from here on is
    ///   verbatim, which is what lets ordinary `send()` stay byte-identical to
    ///   a session without MXP. `ESC[6z` (lock **secure**) is the mirror image
    ///   of this call and is never sent: it would make every unreviewed line of
    ///   game text a markup document.
    /// * `<VERSION>` and `<SUPPORT>`, each on its own secure line. A query is
    ///   itself a tag, so on a locked line it would print. One per line because
    ///   the mode reverts at the newline and does not have to be re-asserted.
    pub fn enable(&mut self) -> Vec<u8> {
        if self.enabled {
            return Vec::new();
        }
        self.enabled = true;

        let mut out = TelnetCodec::encode_subnegotiation(OPT_MXP, &[]);
        out.extend(TelnetCodec::encode_text(&format!(
            "{}{}<VERSION>\n{}<SUPPORT>\n",
            line_tag(MODE_LOCK_LOCKED),
            line_tag(MODE_SECURE),
            line_tag(MODE_SECURE),
        )));
        out
    }

    /// `IAC DONT MXP`, or `WONT`.
    ///
    /// Writes nothing. A client that has just torn its parser down would print
    /// `ESC[3z` as three visible characters, and there is nothing to clean up
    /// on our side: the driver holds no open tags, because the only tags it
    /// ever emits are closed within the line that opened them.
    pub fn disable(&mut self) {
        self.enabled = false;
    }
}

// ─── the security functions ──────────────────────────────────────────────────

/// Remove MXP line-mode sequences from text the driver did not author.
///
/// **This is the load-bearing security function of the whole feature.** The
/// default line mode is locked, so markup in game text is inert — but a mode
/// tag is honoured in every mode, locked included, because that is how a client
/// gets back out of locked mode. A player who types
///
/// ```text
/// say <ESC>[1z<send href="quit">free gold here</send>
/// ```
///
/// has their line round-tripped through the mudlib into everybody else's
/// `SessionOutput::Text`. The `<send>` is not the vulnerability; the `ESC[1z`
/// in front of it is, because it promotes the attacker's own line to secure.
///
/// Only CSI sequences whose final byte is `z` are removed. SGR ends in `m`, so
/// a coloured line is byte-identical to what it was before MXP existed, and a
/// cursor move or an erase reaches the terminal untouched. Returns `Cow` so the
/// overwhelmingly common case — no escape character anywhere — costs a scan and
/// no allocation.
pub fn strip_line_modes(s: &str) -> Cow<'_, str> {
    if !s.as_bytes().contains(&ESC) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == ESC {
            let (seq, next) = escape_sequence(s, i);
            if !is_line_tag(seq) {
                out.push_str(seq);
            }
            i = next;
        } else {
            let start = i;
            i += 1;
            while i < s.len() && s.as_bytes()[i] != ESC {
                i += 1;
            }
            out.push_str(&s[start..i]);
        }
    }
    Cow::Owned(out)
}

/// Make caller-supplied text safe to place inside a line the driver is about to
/// mark secure.
///
/// `<`, `>`, `&` and `"` become their HTML entities; a mode tag is dropped as
/// [`strip_line_modes`] would drop it. Used only by [`crate::core::render`] —
/// never on plain `send()` output, which stays verbatim because its line is
/// locked.
///
/// **An ANSI escape sequence is copied whole rather than examined byte by
/// byte.** The mudlib's colour is not this function's to touch, and a
/// private-mode CSI has parameter bytes in 0x3C–0x3F, which is `<` `=` `>` `?`.
/// Escaping the `<` in `ESC[<0;0;0M` turns a mouse report into a visible
/// `&lt;` and leaves the terminal reading the rest as text.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 8);
    let mut i = 0;
    while i < s.len() {
        match s.as_bytes()[i] {
            ESC => {
                let (seq, next) = escape_sequence(s, i);
                if !is_line_tag(seq) {
                    out.push_str(seq);
                }
                i = next;
            }
            b'<' => {
                out.push_str("&lt;");
                i += 1;
            }
            b'>' => {
                out.push_str("&gt;");
                i += 1;
            }
            b'&' => {
                out.push_str("&amp;");
                i += 1;
            }
            b'"' => {
                out.push_str("&quot;");
                i += 1;
            }
            _ => {
                // A run of ordinary bytes, copied in one go. Byte indexing is
                // safe across UTF-8 here: every continuation byte is >= 0x80
                // and matches no arm above, so a multi-byte character is never
                // split.
                let start = i;
                i += 1;
                while i < s.len()
                    && !matches!(s.as_bytes()[i], ESC | b'<' | b'>' | b'&' | b'"')
                {
                    i += 1;
                }
                out.push_str(&s[start..i]);
            }
        }
    }
    out
}

/// The whole escape sequence beginning at `i`, and the index just past it.
///
/// Deliberately not [`crate::core::network::websocket::protocol`]'s scanner:
/// that one recognises a non-SGR CSI and *drops* it, which is right for a
/// browser that cannot act on a cursor move and wrong here, where the far end
/// is a terminal that can. Two scanners that disagree about where a sequence
/// ends is a real hazard, which is why this one is short enough to check by
/// eye and has its own tests.
///
/// A sequence split across two chunks — the `ESC[` in one `send()` and the `m`
/// in the next — truncates at the chunk boundary and its tail arrives as
/// ordinary text. `Player:_process_output` composes a whole message before
/// calling `send`, so this does not happen in practice; carrying a partial
/// sequence between calls would make every function in this module stateful to
/// guard against something nothing does.
fn escape_sequence(s: &str, i: usize) -> (&str, usize) {
    let b = s.as_bytes();
    if i + 1 < b.len() && b[i + 1] == b'[' {
        // CSI: parameter and intermediate bytes, then a final byte in 0x40-0x7E.
        let mut j = i + 2;
        while j < b.len() && !(0x40..=0x7e).contains(&b[j]) {
            j += 1;
        }
        let end = (j + 1).min(b.len());
        (&s[i..end], end)
    } else {
        // A two-byte sequence (`ESC ( B`), or a lone ESC at the end of a chunk.
        let end = (i + 2).min(b.len());
        (&s[i..end], end)
    }
}

/// `ESC [ <decimal digits> z` — an MXP line-security tag, and nothing else.
fn is_line_tag(seq: &str) -> bool {
    let b = seq.as_bytes();
    b.len() >= 4
        && b[1] == b'['
        && b[b.len() - 1] == b'z'
        && seq[2..seq.len() - 1].bytes().all(|c| c.is_ascii_digit())
}

/// Strip one leading `ESC[<n>z` if there is one, and return the rest.
fn strip_leading_line_tag(s: &str) -> &str {
    if s.starts_with('\x1b') {
        let (seq, next) = escape_sequence(s, 0);
        if is_line_tag(seq) {
            return &s[next..];
        }
    }
    s
}

// ─── the handshake, inbound ──────────────────────────────────────────────────

/// What the client sent back when asked who it is and what it supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MxpReply {
    /// `<VERSION MXP=0.4 STYLE=1 CLIENT=zmud VERSION=6.07 REGISTERED=yes>`
    ///
    /// Attribute pairs in the order they arrived. A `Vec` and not a map: the
    /// tag has a `VERSION` attribute *and* is called `VERSION`, and a map would
    /// invite the confusion as well as throwing the ordering away.
    Version(Vec<(String, String)>),
    /// `<SUPPORTS +b +i +color.fore -image>` — signs kept.
    Supports(Vec<String>),
}

/// Is this input line the client answering a handshake query?
///
/// **The client replies on a secure line**, so the line still carries its
/// leading `ESC[1z` when it reaches here. The spec says so in prose and then
/// omits it from every example, which makes it the easiest detail in MXP to
/// implement against and miss — the reply parses fine and nothing ever matches.
///
/// Returns `None` for anything else, including a player who types `<VERSION>`
/// by hand. There is no way to tell those apart on the wire, and the cost of
/// guessing wrong in this direction is that one typed word is swallowed.
pub fn parse_reply(line: &str) -> Option<MxpReply> {
    let t = strip_leading_line_tag(line.trim());
    let inner = t.strip_prefix('<')?.strip_suffix('>')?;
    let (name, rest) = match inner.find(char::is_whitespace) {
        Some(p) => (&inner[..p], &inner[p..]),
        None => (inner, ""),
    };

    if name.eq_ignore_ascii_case("VERSION") {
        Some(MxpReply::Version(parse_attrs(rest)))
    } else if name.eq_ignore_ascii_case("SUPPORTS") {
        Some(MxpReply::Supports(
            tokenize(rest)
                .into_iter()
                .filter(|t| t.starts_with('+') || t.starts_with('-'))
                .collect(),
        ))
    } else {
        None
    }
}

/// How many `<SUPPORTS>` entries one session may accumulate.
///
/// The list is fed entirely by the client and `<SUPPORT>` may be asked more
/// than once, so without a ceiling a peer can grow it without bound by
/// answering a question nobody asked.
const MAX_SUPPORTS: usize = 256;

/// Copy a reply onto the capability struct.
///
/// `supports` **accumulates** rather than replacing. `<SUPPORT>` can be asked
/// narrowed to particular tags — `<SUPPORT image frame>` — and the answer is
/// about the tags it names and no others. Replacing would make a second,
/// narrower question erase the answer to the first.
pub fn apply(reply: &MxpReply, caps: &mut ClientCapabilities) {
    match reply {
        MxpReply::Version(attrs) => {
            caps.mxp_version = attr(attrs, "MXP").map(str::to_string);
            let client = attr(attrs, "CLIENT").unwrap_or_default();
            let version = attr(attrs, "VERSION").unwrap_or_default();
            let joined = format!("{client} {version}").trim().to_string();
            if !joined.is_empty() {
                caps.mxp_client = Some(joined);
            }
        }
        MxpReply::Supports(tags) => {
            for t in tags {
                if caps.mxp_supports.len() >= MAX_SUPPORTS {
                    break;
                }
                if !caps.mxp_supports.iter().any(|e| e == t) {
                    caps.mxp_supports.push(t.clone());
                }
            }
        }
    }
}

/// Look one attribute up, case-insensitively on the key.
pub fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

/// Split on whitespace, but not inside `'…'` or `"…"`, and drop the quotes.
///
/// MXP quotes an attribute value the way HTML does and the spec's examples use
/// both kinds. `split_whitespace` turns `CLIENT="My Client"` into two tokens,
/// the second of which — `Client"` — is indistinguishable from a flag.
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    // Tracked separately from `cur.is_empty()` so `CLIENT=""` yields a token
    // with an empty value rather than yielding nothing.
    let mut started = false;

    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                started = false;
            }
            None => cur.push(c),
        }
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `KEY=VALUE` pairs; a bare token becomes `(KEY, "")`.
fn parse_attrs(s: &str) -> Vec<(String, String)> {
    tokenize(s)
        .into_iter()
        .map(|t| match t.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (t, String::new()),
        })
        .collect()
}

/// Record a client's acceptance of MXP, and everything that follows from it.
///
/// Split out of `relay.rs`'s negotiation policy so that the *state* changes and
/// the bytes that go with them are described in one place — the enable/disable
/// pair is the only part of negotiation where the two are coupled.
pub fn on_accept(caps: &mut ClientCapabilities, state: &mut MxpState) -> Vec<u8> {
    let start = state.enable();
    if !start.is_empty() {
        caps.mxp_supported = true;
    }
    start
}

/// The client will not parse markup after all.
///
/// Everything derived from the handshake goes with the flag. A stale
/// `mxp_client` would leave the mudlib believing it can still emit links, which
/// is the same failure `publish_capabilities` exists to document, one level in.
pub fn on_refuse(caps: &mut ClientCapabilities, state: &mut MxpState) {
    state.disable();
    caps.mxp_supported = false;
    caps.mxp_version = None;
    caps.mxp_client = None;
    caps.mxp_supports.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_line_modes ────────────────────────────────────────────────────

    #[test]
    fn text_without_an_escape_is_borrowed_unchanged() {
        let s = "You see a <sign> here. Tom & Sons.";
        assert!(matches!(strip_line_modes(s), Cow::Borrowed(_)));
        assert_eq!(strip_line_modes(s), s);
    }

    /// The injection this whole module exists to prevent. A player types the
    /// mode tag into `say`; the mudlib round-trips it into everyone else's
    /// output; without this strip it promotes their line to secure and the
    /// `<send>` behind it becomes a command other players' clients will run.
    #[test]
    fn a_line_mode_sequence_is_stripped_from_game_text() {
        let hostile = "Bob says, '\x1b[1z<send href=\"quit\">free gold here</send>'";
        let safe = strip_line_modes(hostile);
        assert!(!safe.contains('\x1b'), "mode tag survived: {safe:?}");
        // The visible text is untouched — it is a silly thing to say, not a
        // command, and gagging it would be a game's decision and not ours.
        assert!(safe.contains("<send href=\"quit\">free gold here</send>"));
    }

    #[test]
    fn every_mode_tag_goes_including_the_multi_digit_ones() {
        for n in [0u8, 1, 2, 3, 4, 5, 6, 7, 10, 19, 20, 99] {
            let s = format!("a{}b", line_tag(n));
            assert_eq!(strip_line_modes(&s), "ab", "ESC[{n}z survived");
        }
    }

    #[test]
    fn ansi_colour_survives_byte_for_byte() {
        let s = "\x1b[1;31mred\x1b[0m and \x1b[38;5;99mindexed\x1b[0m";
        assert_eq!(strip_line_modes(s), s);
    }

    /// `ESC[?25l` and friends carry `<`, `=`, `>` and `?` as parameter bytes.
    /// A scanner that stopped at the first non-digit would cut one in half.
    #[test]
    fn a_private_mode_csi_is_not_mistaken_for_a_line_tag() {
        let s = "\x1b[?25l\x1b[<0;10;20M";
        assert_eq!(strip_line_modes(s), s);
    }

    #[test]
    fn a_lone_escape_at_the_end_does_not_panic() {
        assert_eq!(strip_line_modes("tail\x1b"), "tail\x1b");
        assert_eq!(strip_line_modes("tail\x1b["), "tail\x1b[");
    }

    // ── escape ──────────────────────────────────────────────────────────────

    #[test]
    fn markup_in_caller_text_becomes_entities() {
        assert_eq!(
            escape("<send href=\"quit\">click</send> & run"),
            "&lt;send href=&quot;quit&quot;&gt;click&lt;/send&gt; &amp; run"
        );
    }

    #[test]
    fn escaping_leaves_ansi_alone_and_still_drops_a_mode_tag() {
        assert_eq!(escape("\x1b[31m<b>\x1b[6z!"), "\x1b[31m&lt;b&gt;!");
    }

    #[test]
    fn escaping_does_not_split_a_multibyte_character() {
        // The run-copying fast path indexes by byte; a continuation byte must
        // never end a run.
        assert_eq!(escape("héllo 🐉 <x>"), "héllo 🐉 &lt;x&gt;");
    }

    // ── MxpState ────────────────────────────────────────────────────────────

    #[test]
    fn enable_sends_the_subnegotiation_then_locks_then_asks() {
        let mut st = MxpState::new(true);
        let out = st.enable();
        assert!(st.is_enabled());

        let head = [IAC, SB, OPT_MXP, IAC, SE];
        assert_eq!(&out[..5], &head, "start subnegotiation missing or misplaced");

        let tail = String::from_utf8(out[5..].to_vec()).unwrap();
        assert_eq!(
            tail,
            "\x1b[7z\x1b[1z<VERSION>\r\n\x1b[1z<SUPPORT>\r\n",
            "the lock must precede the queries, and each query gets its own line"
        );
    }

    /// A repeated `IAC DO 91` must not re-lock the stream and re-ask the
    /// handshake in the middle of whatever the player is reading.
    #[test]
    fn enable_is_empty_the_second_time() {
        let mut st = MxpState::new(true);
        assert!(!st.enable().is_empty());
        assert!(st.enable().is_empty());
    }

    #[test]
    fn disable_writes_nothing_and_re_enable_starts_clean() {
        let mut st = MxpState::new(true);
        st.enable();
        st.disable();
        assert!(!st.is_enabled());
        assert!(!st.enable().is_empty(), "a later DO must start MXP again");
    }

    // ── handshake replies ───────────────────────────────────────────────────

    /// The reply arrives on a secure line, so it still carries `ESC[1z`. Miss
    /// this and nothing ever parses.
    #[test]
    fn the_version_reply_still_carries_its_secure_line_tag() {
        let reply = parse_reply("\x1b[1z<VERSION MXP=0.4 CLIENT=zmud VERSION=6.07>");
        let Some(MxpReply::Version(attrs)) = reply else {
            panic!("did not parse: {reply:?}");
        };
        assert_eq!(attr(&attrs, "MXP"), Some("0.4"));
        assert_eq!(attr(&attrs, "CLIENT"), Some("zmud"));
        assert_eq!(attr(&attrs, "VERSION"), Some("6.07"));
    }

    #[test]
    fn a_quoted_client_name_survives_tokenising() {
        let Some(MxpReply::Version(attrs)) =
            parse_reply(r#"<VERSION CLIENT="My Client" VERSION='1.2'>"#)
        else {
            panic!("did not parse")
        };
        assert_eq!(attr(&attrs, "CLIENT"), Some("My Client"));
        assert_eq!(attr(&attrs, "VERSION"), Some("1.2"));
    }

    #[test]
    fn supports_keeps_the_sign() {
        let Some(MxpReply::Supports(tags)) =
            parse_reply("\x1b[1z<SUPPORTS +b +color.fore -image>")
        else {
            panic!("did not parse")
        };
        assert_eq!(tags, vec!["+b", "+color.fore", "-image"]);
    }

    /// A narrowed `<SUPPORT image frame>` is answered about those tags only.
    /// Replacing rather than accumulating would erase the broad answer.
    #[test]
    fn a_narrower_supports_answer_adds_rather_than_replaces() {
        let mut caps = ClientCapabilities::default();
        apply(&parse_reply("<SUPPORTS +b +i>").unwrap(), &mut caps);
        apply(&parse_reply("<SUPPORTS -image>").unwrap(), &mut caps);
        assert_eq!(caps.mxp_supports, vec!["+b", "+i", "-image"]);
    }

    #[test]
    fn supports_does_not_grow_without_bound() {
        let mut caps = ClientCapabilities::default();
        for i in 0..(MAX_SUPPORTS + 50) {
            apply(&MxpReply::Supports(vec![format!("+t{i}")]), &mut caps);
        }
        assert_eq!(caps.mxp_supports.len(), MAX_SUPPORTS);
    }

    #[test]
    fn version_joins_the_client_name_and_its_version() {
        let mut caps = ClientCapabilities::default();
        apply(
            &parse_reply("<VERSION MXP=0.4 CLIENT=mushclient VERSION=5.06>").unwrap(),
            &mut caps,
        );
        assert_eq!(caps.mxp_client.as_deref(), Some("mushclient 5.06"));
        assert_eq!(caps.mxp_version.as_deref(), Some("0.4"));
    }

    #[test]
    fn ordinary_player_input_is_not_a_handshake_reply() {
        for line in ["look", "say <hello>", "<VERSIONS>", "<", "say i < you", ""] {
            assert_eq!(parse_reply(line), None, "{line:?} was taken for a reply");
        }
    }

    // ── accept / refuse ─────────────────────────────────────────────────────

    #[test]
    fn accepting_records_the_capability_and_starts_the_protocol() {
        let mut state = MxpState::new(true);
        let mut caps = ClientCapabilities::default();
        let out = on_accept(&mut caps, &mut state);
        assert!(caps.mxp_supported);
        assert!(state.is_enabled());
        assert!(out.windows(5).any(|w| w == [IAC, SB, OPT_MXP, IAC, SE]));
        assert!(String::from_utf8_lossy(&out).contains("\x1b[7z"));
    }

    #[test]
    fn refusing_clears_everything_derived_from_the_handshake() {
        let mut state = MxpState::new(true);
        let mut caps = ClientCapabilities::default();

        on_accept(&mut caps, &mut state);
        apply(
            &parse_reply("<VERSION MXP=0.4 CLIENT=zmud VERSION=6.07>").unwrap(),
            &mut caps,
        );
        apply(&parse_reply("<SUPPORTS +b>").unwrap(), &mut caps);
        assert!(caps.mxp_client.is_some());

        on_refuse(&mut caps, &mut state);
        assert!(!state.is_enabled());
        assert!(!caps.mxp_supported);
        assert_eq!(caps.mxp_client, None);
        assert_eq!(caps.mxp_version, None);
        assert!(caps.mxp_supports.is_empty());
    }
}
