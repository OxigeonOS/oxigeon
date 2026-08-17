//! A line of game text, authored once and rendered per viewer.
//!
//! Ordinary output is a string: the mudlib composes it, colours it, wraps it,
//! and the driver moves the bytes. That works because every viewer wants the
//! same thing — characters. It stops working the moment a line carries an
//! *action*: "clicking this word sends `buy bread`" is not something a string
//! can say to a telnet client, a browser and a log file at once.
//!
//! So a rich line arrives here as a tree and chooses its rendering at the
//! transport, where the driver knows what the far end can read. Telnet with MXP
//! negotiated gets [`to_mxp`]; everything else gets [`to_text`], which is the
//! same line with the affordance dropped and not one character of prose lost.
//!
//! # Why a tree and not a string of markup
//!
//! The alternative — an efun taking a finished `<SEND href="...">` string — was
//! rejected, and the reason is the whole design:
//!
//! > Be absolutely sure that MUD players cannot control the output of a secure
//! > line.  — the MXP 1.0 specification
//!
//! A MUD's entire job is showing one player's text to another. Item names, mob
//! names, descriptions and channel messages are all reachable by somebody, so
//! "remember to escape it" is a rule that holds until the hundredth call site.
//! With a tree, [`Node::Text`] holds **only literal text** and there is no
//! constructor that takes markup, so the escaping is not a discipline anyone
//! can forget — [`to_mxp`] is the only function in the driver that emits a `<`,
//! and everything that reaches it goes through
//! [`crate::core::network::telnet::mxp::escape`] on the way.
//!
//! # What is deliberately absent
//!
//! **Styling.** No `fg`, no `bold`. The mudlib already owns colour — `{colour}`
//! tags become ANSI in Lua, long before the driver sees a byte — and MXP is
//! explicit that ANSI keeps working alongside it. A `Text` node carries
//! whatever escape sequences the mudlib put there and the escaper steps over
//! them whole. Adding colour here would mean a second colour system that
//! disagreed with the first about what `red` means.
//!
//! **Wrapping.** `strings.wrap` and `wrap_tagged` decide where lines break, in
//! Lua, using the width `get_session` reports. A second wrapper here would be a
//! mirror of that one, and the cost of not having it is honest and small: a
//! rich line the mudlib did not pre-wrap will not wrap around a `<SEND>`.

use crate::core::network::telnet::mxp::{self, LineMode};

/// One piece of an authored line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// Literal text. **Never markup** — there is no constructor that accepts
    /// any, which is what makes the escaping structural rather than a habit.
    Text(String),
    /// A run of children sharing one action.
    Group(Group),
    /// A hard line break inside the line: `<BR>` under MXP, `\n` otherwise.
    Break,
    /// `<EXPIRE name>` — retire the links previously tagged with that name, or
    /// every named link if there is no name. A room's exits stop being
    /// clickable once the player has left the room.
    ///
    /// Renders to nothing at all for a viewer without MXP, which is correct:
    /// there was no link to retire.
    Expire(Option<String>),
}

/// A run of text that does something when clicked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Group {
    pub children: Vec<Node>,
    /// One command, or several — several is a popup menu, which MXP encodes as
    /// a `|`-separated list. An element containing `|` or `"` is refused when
    /// the tree is built rather than mangled here: there is no escape for `|`
    /// inside a `<SEND>` menu, so silently splitting one command into two would
    /// send the player something nobody wrote.
    pub send: Vec<String>,
    /// Mouse-over text. With a menu, element 0 is the menu's own caption and
    /// the rest label its items.
    pub hint: Vec<String>,
    /// A URL, for `<A href=…>`. Scheme-restricted where the tree is built.
    pub href: Option<String>,
    /// Put the command on the player's input line instead of running it.
    pub prompt: bool,
    /// Name this link belongs to, for a later [`Node::Expire`].
    pub expire: Option<String>,
    /// Also store the rendered text in a client-side variable of this name:
    /// `<VAR hp>40</VAR>`. Displayed either way — the point of `<VAR>` over
    /// `<!ENTITY>` is that the player sees the value as well as the client.
    pub var: Option<String>,
}

impl Group {
    /// Whether this group needs a link tag at all, or is just a container.
    fn is_actionable(&self) -> bool {
        !self.send.is_empty() || self.href.is_some()
    }
}

/// A whole authored line, before any transport has looked at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichLine {
    pub nodes: Vec<Node>,
    /// The MXP line-security mode. Only [`to_mxp`] reads it.
    ///
    /// Defaults to [`LineMode::Secure`], and that is safe for the same reason
    /// the rest of this module is: every byte of markup on the line was written
    /// by [`to_mxp`], and every byte of content was escaped on the way in. The
    /// mode reverts to the connection's locked default at the line's own
    /// newline, so no caller can leave the stream secure.
    pub mode: LineMode,
    /// A line category the client can style, gag or route: 10/11/12 for a room
    /// name, description and exits, 19 for welcome text, or 20-99 for whatever
    /// a game defines. Replaces the mode tag when present.
    pub tag: Option<u8>,
    /// Whether to terminate the line. `false` for a prompt, so the cursor stays
    /// put.
    ///
    /// A rich line terminates *itself*, where `send` leaves that to the mudlib.
    /// It has to: the line boundary is where an MXP mode reverts, so it is a
    /// property the renderer needs rather than a character in the content.
    pub newline: bool,
}

impl RichLine {
    pub fn new(nodes: Vec<Node>) -> Self {
        RichLine { nodes, mode: LineMode::Secure, tag: None, newline: true }
    }
}

/// Render for a client that has MXP live.
///
/// **The only function in the driver permitted to emit a `<`.** Every string
/// that reaches the output from the caller goes through [`mxp::escape`], and
/// every attribute value is quoted, so a mob named
/// `"><send href="quit">gold</send>` is a preposterous name and nothing else.
///
/// The mode prefix is re-asserted after every interior newline, because MXP
/// modes revert at the newline the client reads. That is the transform the
/// mudlib structurally cannot do: by the time Lua has finished composing a
/// message it is a string, and how many lines it will occupy is a question
/// about the client's parser rather than about the text.
pub fn to_mxp(line: &RichLine) -> String {
    let prefix = mxp::line_tag(line.tag.unwrap_or_else(|| line.mode.code()));
    let mut out = String::with_capacity(64);
    out.push_str(&prefix);
    for node in &line.nodes {
        write_mxp(node, &prefix, &mut out);
    }
    if line.newline {
        out.push('\n');
    }
    out
}

fn write_mxp(node: &Node, prefix: &str, out: &mut String) {
    match node {
        Node::Text(t) => {
            // A newline inside the content ends the client's line, and with it
            // the mode. Re-assert immediately so the remainder is parsed the
            // same way the beginning was.
            let escaped = mxp::escape(t);
            let mut first = true;
            for part in escaped.split('\n') {
                if !first {
                    out.push('\n');
                    out.push_str(prefix);
                }
                out.push_str(part);
                first = false;
            }
        }
        Node::Break => out.push_str("<BR>"),
        Node::Expire(name) => match name {
            Some(n) => {
                out.push_str("<EXPIRE ");
                out.push_str(&mxp::escape(n));
                out.push('>');
            }
            None => out.push_str("<EXPIRE>"),
        },
        Node::Group(g) => {
            // `<VAR>` wraps whatever the link tag produces, so the value the
            // client stores is the text the player sees and not the markup
            // around it.
            if let Some(name) = &g.var {
                out.push_str("<VAR ");
                out.push_str(&mxp::escape(name));
                out.push('>');
            }
            write_mxp_link(g, prefix, out);
            if g.var.is_some() {
                out.push_str("</VAR>");
            }
        }
    }
}

/// `<SEND …>` or `<A …>` around the group's children, or just the children if
/// the group carries no action.
fn write_mxp_link(g: &Group, prefix: &str, out: &mut String) {
    if !g.is_actionable() {
        for child in &g.children {
            write_mxp(child, prefix, out);
        }
        return;
    }
    // `href` wins over `send`, which is checked where the tree is built: a
    // group that carried both would be a link that did two different things
    // depending on which client read it.
    let (tag, target) = match &g.href {
        Some(url) => ("A", url.clone()),
        None => ("SEND", g.send.join("|")),
    };
    out.push('<');
    out.push_str(tag);
    out.push_str(" href=\"");
    out.push_str(&mxp::escape(&target));
    out.push('"');
    if !g.hint.is_empty() {
        out.push_str(" hint=\"");
        out.push_str(&mxp::escape(&g.hint.join("|")));
        out.push('"');
    }
    if let Some(name) = &g.expire {
        out.push_str(" expire=\"");
        out.push_str(&mxp::escape(name));
        out.push('"');
    }
    // A valueless attribute: the client puts the command on the input line
    // rather than running it. Meaningless on `<A>`, which opens a browser.
    if g.prompt && g.href.is_none() {
        out.push_str(" PROMPT");
    }
    out.push('>');
    for child in &g.children {
        write_mxp(child, prefix, out);
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// Render for everyone else: the visible text, with whatever ANSI the mudlib
/// put in it, and the actions dropped.
///
/// This is what a plain telnet client sees, what the WebSocket envelope
/// carries, and what the line means with no client at all. The degradation is
/// exactly one lost affordance — a word that would have been clickable is
/// simply a word — which is why a rich line is safe to send unconditionally.
pub fn to_text(line: &RichLine) -> String {
    let mut out = String::with_capacity(64);
    for node in &line.nodes {
        write_text(node, &mut out);
    }
    if line.newline {
        out.push('\n');
    }
    out
}

fn write_text(node: &Node, out: &mut String) {
    match node {
        Node::Text(t) => out.push_str(t),
        Node::Break => out.push('\n'),
        // Nothing. A viewer with no MXP had no link to retire.
        Node::Expire(_) => {}
        Node::Group(g) => {
            for child in &g.children {
                write_text(child, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Node {
        Node::Text(s.to_string())
    }

    fn link(cmd: &str, label: &str) -> Node {
        Node::Group(Group {
            children: vec![text(label)],
            send: vec![cmd.to_string()],
            ..Default::default()
        })
    }

    #[test]
    fn a_send_link_becomes_a_secure_line() {
        let line = RichLine::new(vec![
            text("The baker offers "),
            link("buy bread", "a warm loaf"),
            text("."),
        ]);
        assert_eq!(
            to_mxp(&line),
            "\x1b[1zThe baker offers <SEND href=\"buy bread\">a warm loaf</SEND>.\n"
        );
    }

    /// The whole reason this module is a tree. `mob.short` is attacker-supplied
    /// in any game where a player can name something.
    #[test]
    fn a_hostile_name_cannot_escape_its_group() {
        let hostile = "\"><send href=\"quit\">free gold</send>";
        let line = RichLine::new(vec![link("look", hostile)]);
        let out = to_mxp(&line);

        // Exactly one opening and one closing SEND: the driver's own.
        assert_eq!(out.matches("<SEND").count(), 1);
        assert_eq!(out.matches("</SEND>").count(), 1);
        // And nothing of the payload survives as markup.
        assert!(!out.contains("href=\"quit\""));
        assert!(out.contains("&lt;send"));
    }

    #[test]
    fn a_hostile_command_cannot_escape_its_attribute() {
        let line = RichLine::new(vec![Node::Group(Group {
            children: vec![text("click")],
            send: vec!["look\"><b".to_string()],
            ..Default::default()
        })]);
        let out = to_mxp(&line);
        assert!(out.contains("href=\"look&quot;&gt;&lt;b\""), "{out}");
    }

    /// A mode tag reverts at the newline the client reads, so a multi-line
    /// message has to re-assert it or everything after the first line is
    /// parsed in the connection's locked default.
    #[test]
    fn the_mode_is_reasserted_after_every_interior_newline() {
        let line = RichLine::new(vec![text("one\ntwo\nthree")]);
        assert_eq!(to_mxp(&line), "\x1b[1zone\n\x1b[1ztwo\n\x1b[1zthree\n");
    }

    #[test]
    fn a_line_tag_replaces_the_mode_tag() {
        let mut line = RichLine::new(vec![text("The Main Temple")]);
        line.tag = Some(mxp::MODE_ROOM_NAME);
        assert_eq!(to_mxp(&line), "\x1b[10zThe Main Temple\n");
    }

    /// A prompt exists to leave the cursor where it is, so neither rendering
    /// may append a terminator. Note the `&gt;`: the caret in `> ` is content
    /// like any other and comes back out of the client's parser as itself.
    #[test]
    fn a_prompt_gets_no_terminator() {
        let mut line = RichLine::new(vec![text("> ")]);
        line.newline = false;
        assert_eq!(to_mxp(&line), "\x1b[1z&gt; ");
        assert_eq!(to_text(&line), "> ");
    }

    #[test]
    fn a_menu_joins_its_commands_and_hints() {
        let line = RichLine::new(vec![Node::Group(Group {
            children: vec![text("the counter")],
            send: vec!["buy bread".into(), "buy cake".into()],
            hint: vec!["Shop".into(), "Bread".into(), "Cake".into()],
            ..Default::default()
        })]);
        assert!(to_mxp(&line)
            .contains("<SEND href=\"buy bread|buy cake\" hint=\"Shop|Bread|Cake\">"));
    }

    #[test]
    fn a_url_becomes_an_anchor_and_never_a_send() {
        let line = RichLine::new(vec![Node::Group(Group {
            children: vec![text("the site")],
            href: Some("https://example.invalid/x?a=1".into()),
            ..Default::default()
        })]);
        let out = to_mxp(&line);
        assert!(out.contains("<A href=\"https://example.invalid/x?a=1\">the site</A>"));
        assert!(!out.contains("SEND"));
    }

    #[test]
    fn a_group_with_no_action_is_just_its_children() {
        let line = RichLine::new(vec![Node::Group(Group {
            children: vec![text("plain")],
            ..Default::default()
        })]);
        assert_eq!(to_mxp(&line), "\x1b[1zplain\n");
    }

    #[test]
    fn to_text_keeps_the_prose_and_the_colour_and_drops_the_link() {
        let line = RichLine::new(vec![
            text("\x1b[31mThe baker\x1b[0m offers "),
            link("buy bread", "a warm loaf"),
            text("."),
        ]);
        assert_eq!(to_text(&line), "\x1b[31mThe baker\x1b[0m offers a warm loaf.\n");
    }

    #[test]
    fn a_break_is_a_tag_for_mxp_and_a_newline_for_everyone_else() {
        let line = RichLine::new(vec![text("a"), Node::Break, text("b")]);
        assert_eq!(to_mxp(&line), "\x1b[1za<BR>b\n");
        assert_eq!(to_text(&line), "a\nb\n");
    }

    /// The invariant that catches a renderer drifting from its sibling: strip
    /// the markup back out of the MXP rendering and the prose must be what the
    /// plain rendering says it is. One authored line, read per viewer.
    #[test]
    fn both_renderings_agree_about_the_prose() {
        let cases: Vec<RichLine> = vec![
            RichLine::new(vec![text("plain text")]),
            RichLine::new(vec![text("a & b < c > d \" e")]),
            RichLine::new(vec![text("before "), link("look", "here"), text(" after")]),
            RichLine::new(vec![text("multi\nline\ntext")]),
            RichLine::new(vec![text("\x1b[1;31mcoloured\x1b[0m")]),
            RichLine::new(vec![text("héllo 🐉")]),
            RichLine::new(vec![link("x", "<send href=\"quit\">hostile</send>")]),
        ];
        for line in &cases {
            assert_eq!(
                unmarkup(&to_mxp(line)),
                to_text(line),
                "renderings disagree for {line:?}"
            );
        }
    }

    /// Undo `to_mxp` far enough to compare prose: drop the line tags, drop the
    /// tags the renderer emits, and resolve the entities it writes. Test-only,
    /// and deliberately not a general MXP parser — the driver has no business
    /// owning one of those.
    fn unmarkup(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find(['\x1b', '<']) {
            out.push_str(&rest[..i]);
            rest = &rest[i..];
            let end = if rest.starts_with('\x1b') {
                rest.find('z').map(|j| j + 1)
            } else {
                rest.find('>').map(|j| j + 1)
            };
            match end {
                Some(j) => {
                    if rest.starts_with("<BR>") {
                        out.push('\n');
                    }
                    rest = &rest[j..];
                }
                None => break,
            }
        }
        out.push_str(rest);
        out.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
    }
}
