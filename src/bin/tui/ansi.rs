//! ANSI SGR → `ratatui` spans.
//!
//! `mudlib/lib/color.lua` emits a known, small set: `ESC[Nm` for the 16 colours,
//! the bright pair, the styles and reset, plus `ESC[38;5;Nm` / `ESC[48;5;Nm` for
//! its xterm-256 aliases. Everything here covers that, plus 24-bit colour and
//! the "turn it off again" codes for completeness, and ignores any other CSI
//! sequence rather than printing it.
//!
//! The decoder is a byte state machine because output arrives in arbitrary TCP
//! chunks: an escape sequence, a UTF-8 character, or a line can be split across
//! two reads. Style carries across line breaks, as it does on a real terminal.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Normal,
    Escape,
    Csi,
}

#[derive(Default)]
pub struct AnsiDecoder {
    state: State,
    /// Parameter bytes of the CSI sequence currently being read.
    params: Vec<u8>,
    /// Style in force, carried across lines.
    style: Style,
    /// Spans completed on the line being built.
    spans: Vec<Span<'static>>,
    /// Bytes of the span currently being built, decoded when the span closes.
    text: Vec<u8>,
}

impl Default for State {
    fn default() -> Self {
        State::Normal
    }
}

impl AnsiDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of game output. Returns every line completed by it; any
    /// trailing partial line stays buffered and is readable via [`partial`].
    ///
    /// [`partial`]: AnsiDecoder::partial
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for &b in bytes {
            match self.state {
                State::Normal => match b {
                    0x1b => self.state = State::Escape,
                    b'\n' => lines.push(self.take_line()),
                    // The mudlib sends CRLF; a bare CR is a carriage return we
                    // have no use for either, since we rebuild the line anyway.
                    b'\r' => {}
                    // Telnet NUL padding after CR, and the BEL we do not ring.
                    0x00 | 0x07 => {}
                    _ => self.text.push(b),
                },
                State::Escape => {
                    if b == b'[' {
                        self.params.clear();
                        self.state = State::Csi;
                    } else {
                        // Not a CSI — ESC ) 0 and friends. Drop the introducer
                        // and treat the next byte as ordinary text.
                        self.state = State::Normal;
                    }
                }
                State::Csi => {
                    // A CSI ends at the first byte in 0x40..=0x7e; everything
                    // before it is parameter or intermediate bytes.
                    if (0x40..=0x7e).contains(&b) {
                        if b == b'm' {
                            self.apply_sgr();
                        }
                        self.state = State::Normal;
                    } else {
                        self.params.push(b);
                    }
                }
            }
        }
        lines
    }

    /// The partial line currently buffered — the prompt, which the driver sends
    /// without a trailing newline. Returns `None` when nothing is pending.
    pub fn partial(&self) -> Option<Line<'static>> {
        if self.text.is_empty() && self.spans.is_empty() {
            return None;
        }
        let mut spans = self.spans.clone();
        if !self.text.is_empty() {
            spans.push(Span::styled(
                String::from_utf8_lossy(&self.text).into_owned(),
                self.style,
            ));
        }
        Some(Line::from(spans))
    }

    fn close_span(&mut self) {
        if !self.text.is_empty() {
            let text = String::from_utf8_lossy(&self.text).into_owned();
            self.spans.push(Span::styled(text, self.style));
            self.text.clear();
        }
    }

    fn take_line(&mut self) -> Line<'static> {
        self.close_span();
        Line::from(std::mem::take(&mut self.spans))
    }

    fn apply_sgr(&mut self) {
        // An empty parameter list — a bare `ESC[m` — means reset, same as `0`.
        let raw = String::from_utf8_lossy(&self.params);
        let codes: Vec<u16> = if raw.is_empty() {
            vec![0]
        } else {
            raw.split(';')
                .map(|p| p.trim().parse::<u16>().unwrap_or(0))
                .collect()
        };

        let before = self.style;
        let mut i = 0;
        while i < codes.len() {
            match codes[i] {
                0 => self.style = Style::default(),
                1 => self.style = self.style.add_modifier(Modifier::BOLD),
                2 => self.style = self.style.add_modifier(Modifier::DIM),
                3 => self.style = self.style.add_modifier(Modifier::ITALIC),
                4 => self.style = self.style.add_modifier(Modifier::UNDERLINED),
                5 => self.style = self.style.add_modifier(Modifier::SLOW_BLINK),
                7 => self.style = self.style.add_modifier(Modifier::REVERSED),
                9 => self.style = self.style.add_modifier(Modifier::CROSSED_OUT),
                22 => {
                    self.style = self
                        .style
                        .remove_modifier(Modifier::BOLD | Modifier::DIM)
                }
                23 => self.style = self.style.remove_modifier(Modifier::ITALIC),
                24 => self.style = self.style.remove_modifier(Modifier::UNDERLINED),
                25 => self.style = self.style.remove_modifier(Modifier::SLOW_BLINK),
                27 => self.style = self.style.remove_modifier(Modifier::REVERSED),
                29 => self.style = self.style.remove_modifier(Modifier::CROSSED_OUT),
                c @ 30..=37 => self.style = self.style.fg(basic(c - 30)),
                38 => {
                    if let Some((color, used)) = extended(&codes[i..]) {
                        self.style = self.style.fg(color);
                        i += used - 1;
                    }
                }
                39 => self.style = self.style.fg(Color::Reset),
                c @ 40..=47 => self.style = self.style.bg(basic(c - 40)),
                48 => {
                    if let Some((color, used)) = extended(&codes[i..]) {
                        self.style = self.style.bg(color);
                        i += used - 1;
                    }
                }
                49 => self.style = self.style.bg(Color::Reset),
                c @ 90..=97 => self.style = self.style.fg(bright(c - 90)),
                c @ 100..=107 => self.style = self.style.bg(bright(c - 100)),
                // Anything else is a code the mudlib does not emit; ignoring it
                // is better than rendering the escape as text.
                _ => {}
            }
            i += 1;
        }

        // Only break the span if the style actually moved. A no-op `ESC[m` in
        // the middle of a word should not fragment it.
        if self.style != before {
            let after = self.style;
            self.style = before;
            self.close_span();
            self.style = after;
        }
    }
}

/// `38`/`48` take a sub-form: `;5;N` for indexed, `;2;R;G;B` for 24-bit.
/// Returns the colour and how many codes it consumed.
fn extended(codes: &[u16]) -> Option<(Color, usize)> {
    match codes.get(1)? {
        5 => Some((Color::Indexed(*codes.get(2)? as u8), 3)),
        2 => {
            let r = *codes.get(2)? as u8;
            let g = *codes.get(3)? as u8;
            let b = *codes.get(4)? as u8;
            Some((Color::Rgb(r, g, b), 5))
        }
        _ => None,
    }
}

fn basic(n: u16) -> Color {
    match n {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::Gray,
    }
}

fn bright(n: u16) -> Color {
    match n {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        _ => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flatten a line to `(text, style)` pairs for assertions.
    fn parts(line: &Line<'static>) -> Vec<(String, Style)> {
        line.spans
            .iter()
            .map(|s| (s.content.to_string(), s.style))
            .collect()
    }

    #[test]
    fn plain_text_splits_on_newlines_and_drops_cr() {
        let mut d = AnsiDecoder::new();
        let lines = d.feed(b"one\r\ntwo\r\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(parts(&lines[0])[0].0, "one");
        assert_eq!(parts(&lines[1])[0].0, "two");
        assert!(d.partial().is_none());
    }

    #[test]
    fn a_trailing_partial_line_is_the_prompt() {
        let mut d = AnsiDecoder::new();
        let lines = d.feed(b"scrolled\r\n42h 20m> ");
        assert_eq!(lines.len(), 1);
        let prompt = d.partial().expect("prompt should be buffered");
        assert_eq!(parts(&prompt)[0].0, "42h 20m> ");
    }

    #[test]
    fn colour_codes_become_styled_spans() {
        let mut d = AnsiDecoder::new();
        // What `{red}danger{/} ok` colorizes to.
        let lines = d.feed(b"\x1b[31mdanger\x1b[0m ok\r\n");
        let p = parts(&lines[0]);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].0, "danger");
        assert_eq!(p[0].1.fg, Some(Color::Red));
        assert_eq!(p[1].0, " ok");
        assert_eq!(p[1].1.fg, None);
    }

    #[test]
    fn xterm_256_foreground_and_background() {
        let mut d = AnsiDecoder::new();
        // `{orange}` is 208; `{bg:17}` is the midnight background.
        let lines = d.feed(b"\x1b[38;5;208mA\x1b[48;5;17mB\r\n");
        let p = parts(&lines[0]);
        assert_eq!(p[0].1.fg, Some(Color::Indexed(208)));
        assert_eq!(p[1].1.fg, Some(Color::Indexed(208)));
        assert_eq!(p[1].1.bg, Some(Color::Indexed(17)));
    }

    #[test]
    fn truecolor_is_understood_even_though_the_mudlib_does_not_emit_it() {
        let mut d = AnsiDecoder::new();
        let lines = d.feed(b"\x1b[38;2;10;20;30mx\r\n");
        assert_eq!(parts(&lines[0])[0].1.fg, Some(Color::Rgb(10, 20, 30)));
    }

    #[test]
    fn modifiers_accumulate_and_clear() {
        let mut d = AnsiDecoder::new();
        let lines = d.feed(b"\x1b[1m\x1b[4mboth\x1b[24monly bold\r\n");
        let p = parts(&lines[0]);
        assert!(p[0].1.add_modifier.contains(Modifier::BOLD));
        assert!(p[0].1.add_modifier.contains(Modifier::UNDERLINED));
        assert!(p[1].1.add_modifier.contains(Modifier::BOLD));
        assert!(!p[1].1.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn a_sequence_split_across_reads_still_parses() {
        // The failure this guards: an escape straddling a TCP chunk boundary
        // rendering as literal "[31m" in the game pane.
        let mut d = AnsiDecoder::new();
        assert!(d.feed(b"\x1b[3").is_empty());
        assert!(d.feed(b"1mred").is_empty());
        let lines = d.feed(b"\r\n");
        let p = parts(&lines[0]);
        assert_eq!(p[0].0, "red");
        assert_eq!(p[0].1.fg, Some(Color::Red));
    }

    #[test]
    fn a_utf8_character_split_across_reads_survives() {
        let mut d = AnsiDecoder::new();
        // The box-drawing the trace output uses, cut mid-codepoint.
        let full = "──".as_bytes();
        assert!(d.feed(&full[..3]).is_empty());
        let lines = d.feed(&[&full[3..], b"\r\n" as &[u8]].concat());
        assert_eq!(parts(&lines[0])[0].0, "──");
    }

    #[test]
    fn style_carries_across_a_line_break() {
        let mut d = AnsiDecoder::new();
        let lines = d.feed(b"\x1b[32mgreen\r\nstill green\r\n");
        assert_eq!(parts(&lines[0])[0].1.fg, Some(Color::Green));
        assert_eq!(parts(&lines[1])[0].1.fg, Some(Color::Green));
    }

    #[test]
    fn non_sgr_csi_sequences_are_swallowed_not_printed() {
        let mut d = AnsiDecoder::new();
        let lines = d.feed(b"a\x1b[2Jb\r\n");
        assert_eq!(parts(&lines[0])[0].0, "ab");
    }

    #[test]
    fn a_no_op_reset_does_not_fragment_a_word() {
        let mut d = AnsiDecoder::new();
        let lines = d.feed(b"unbro\x1b[0mken\r\n");
        assert_eq!(parts(&lines[0]).len(), 1);
    }
}
