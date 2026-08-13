//! Just enough Lua to colour it.
//!
//! Not a parser and not trying to be. The source pane shows a file you are
//! reading, not one you are editing, so this only has to be right about the
//! things that make reading hard: where a string ends, where a comment ends, and
//! which words are structure rather than names.
//!
//! Long brackets (`[[ ... ]]`, `--[==[ ... ]==]`) are the one construct that
//! cannot be decided a line at a time — a `"` inside one is not a string, and a
//! keyword inside one is not a keyword. [`block_state`] scans the file once when
//! it is opened and records, per line, whether it begins inside one; the
//! per-line tokenizer takes that as its starting condition. Doing it per frame
//! instead would rescan the whole file forty times a redraw.

/// What a run of characters is, for colouring.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Plain,
    Keyword,
    /// `nil`, `true`, `false`, and numbers.
    Literal,
    Str,
    Comment,
    /// The name after `function`, `local function`, or a `:`/`.` call.
    Ident,
}

/// Reserved words. `self` is not one, but reads as structure and is coloured
/// like it — the alternative is that the most important name in a method body
/// looks like every other local.
const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "self",
];

const LITERALS: &[&str] = &["nil", "true", "false"];

/// Whether each line *starts* inside a long bracket, and at what level.
///
/// `None` means ordinary code. `Some(level)` means inside `[=*[` with that many
/// `=` signs, so only a matching `]=*]` closes it.
pub fn block_state(lines: &[String]) -> Vec<Option<usize>> {
    let mut out = Vec::with_capacity(lines.len());
    let mut open: Option<usize> = None;
    for line in lines {
        out.push(open);
        open = scan_line_blocks(line, open);
    }
    out
}

/// Follow one line's long-bracket transitions, returning the state after it.
fn scan_line_blocks(line: &str, mut open: Option<usize>) -> Option<usize> {
    let b: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < b.len() {
        match open {
            Some(level) => {
                if let Some(n) = close_at(&b, i) {
                    if n == level {
                        open = None;
                        i += level + 2;
                        continue;
                    }
                }
                i += 1;
            }
            None => {
                // A short comment eats the rest of the line — unless it opens a
                // long one, which `--[[` does.
                if b[i] == '-' && b.get(i + 1) == Some(&'-') {
                    if let Some(n) = open_at(&b, i + 2) {
                        open = Some(n);
                        i += n + 4;
                        continue;
                    }
                    return None;
                }
                if let Some(n) = open_at(&b, i) {
                    open = Some(n);
                    i += n + 2;
                    continue;
                }
                // A quoted string cannot contain a long bracket opener that
                // matters, so skip over it wholesale.
                if b[i] == '"' || b[i] == '\'' {
                    i = skip_quoted(&b, i);
                    continue;
                }
                i += 1;
            }
        }
    }
    open
}

/// `[`, `n` equals signs, `[` — returns `n`.
fn open_at(b: &[char], i: usize) -> Option<usize> {
    if b.get(i) != Some(&'[') {
        return None;
    }
    let mut n = 0;
    while b.get(i + 1 + n) == Some(&'=') {
        n += 1;
    }
    (b.get(i + 1 + n) == Some(&'[')).then_some(n)
}

/// `]`, `n` equals signs, `]` — returns `n`.
fn close_at(b: &[char], i: usize) -> Option<usize> {
    if b.get(i) != Some(&']') {
        return None;
    }
    let mut n = 0;
    while b.get(i + 1 + n) == Some(&'=') {
        n += 1;
    }
    (b.get(i + 1 + n) == Some(&']')).then_some(n)
}

/// Index just past a `"`- or `'`-quoted string starting at `i`.
fn skip_quoted(b: &[char], i: usize) -> usize {
    let quote = b[i];
    let mut j = i + 1;
    while j < b.len() {
        if b[j] == '\\' {
            j += 2;
            continue;
        }
        if b[j] == quote {
            return j + 1;
        }
        j += 1;
    }
    // Unterminated: Lua would reject the file, so treat it as ending here
    // rather than swallowing everything after it.
    b.len()
}

/// Split one line into `(text, kind)` runs.
///
/// `starts_in` comes from [`block_state`] and says whether this line begins
/// inside a long bracket.
pub fn tokenize(line: &str, starts_in: Option<usize>) -> Vec<(String, Tok)> {
    let b: Vec<char> = line.chars().collect();
    let mut out: Vec<(String, Tok)> = Vec::new();
    let mut i = 0;

    // Everything up to the closing bracket belongs to the block that was open
    // when the line started.
    if let Some(level) = starts_in {
        let mut j = 0;
        while j < b.len() {
            if close_at(&b, j) == Some(level) {
                j += level + 2;
                break;
            }
            j += 1;
        }
        push(&mut out, &b[..j.min(b.len())], Tok::Comment);
        i = j.min(b.len());
    }

    while i < b.len() {
        let c = b[i];

        // ── comments ────────────────────────────────────────────────────
        if c == '-' && b.get(i + 1) == Some(&'-') {
            if let Some(n) = open_at(&b, i + 2) {
                let mut j = i + n + 4;
                while j < b.len() && close_at(&b, j) != Some(n) {
                    j += 1;
                }
                let end = (j + n + 2).min(b.len());
                push(&mut out, &b[i..end], Tok::Comment);
                i = end;
                continue;
            }
            push(&mut out, &b[i..], Tok::Comment);
            break;
        }

        // ── strings ─────────────────────────────────────────────────────
        if c == '"' || c == '\'' {
            let end = skip_quoted(&b, i);
            push(&mut out, &b[i..end], Tok::Str);
            i = end;
            continue;
        }
        if let Some(n) = open_at(&b, i) {
            let mut j = i + n + 2;
            while j < b.len() && close_at(&b, j) != Some(n) {
                j += 1;
            }
            let end = (j + n + 2).min(b.len());
            push(&mut out, &b[i..end], Tok::Str);
            i = end;
            continue;
        }

        // ── numbers ─────────────────────────────────────────────────────
        if c.is_ascii_digit() {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == '.') {
                j += 1;
            }
            push(&mut out, &b[i..j], Tok::Literal);
            i = j;
            continue;
        }

        // ── words ───────────────────────────────────────────────────────
        if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < b.len() && (b[j].is_alphanumeric() || b[j] == '_') {
                j += 1;
            }
            let word: String = b[i..j].iter().collect();
            let kind = if LITERALS.contains(&word.as_str()) {
                Tok::Literal
            } else if KEYWORDS.contains(&word.as_str()) {
                Tok::Keyword
            } else if is_call(&b, j) {
                // Followed by `(` — a call or a definition, and the thing you
                // scan for when you are looking for where something happens.
                Tok::Ident
            } else {
                Tok::Plain
            };
            push(&mut out, &b[i..j], kind);
            i = j;
            continue;
        }

        push(&mut out, &b[i..i + 1], Tok::Plain);
        i += 1;
    }

    out
}

/// Whether the next non-space character opens a call.
fn is_call(b: &[char], from: usize) -> bool {
    b[from..].iter().find(|c| !c.is_whitespace()) == Some(&'(')
}

/// Append a run, merging it into the previous one when the kind matches — fewer
/// spans is less work for the renderer and less noise in a test's output.
fn push(out: &mut Vec<(String, Tok)>, chars: &[char], kind: Tok) {
    if chars.is_empty() {
        return;
    }
    let text: String = chars.iter().collect();
    match out.last_mut() {
        Some((prev, k)) if *k == kind => prev.push_str(&text),
        _ => out.push((text, kind)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<(String, Tok)> {
        tokenize(line, None)
    }

    /// The kind of the run that is exactly `text`.
    ///
    /// Exact, not containment: adjacent runs of the same kind are merged, so
    /// `(session_id)` is one `Plain` run and asking for `session_id` on its own
    /// would find nothing. Use [`within`] when the run is a merged one.
    fn find(toks: &[(String, Tok)], text: &str) -> Option<Tok> {
        toks.iter().find(|(t, _)| t == text).map(|(_, k)| *k)
    }

    /// The kind of the run *containing* `text`.
    fn within(toks: &[(String, Tok)], text: &str) -> Option<Tok> {
        toks.iter().find(|(t, _)| t.contains(text)).map(|(_, k)| *k)
    }

    #[test]
    fn keywords_and_names_are_told_apart() {
        let t = kinds("local function search_entrance(session_id)");
        assert_eq!(find(&t, "local"), Some(Tok::Keyword));
        assert_eq!(find(&t, "function"), Some(Tok::Keyword));
        assert_eq!(find(&t, "search_entrance"), Some(Tok::Ident));
        assert_eq!(within(&t, "session_id"), Some(Tok::Plain));
    }

    #[test]
    fn nil_true_and_numbers_are_literals() {
        let t = kinds("if x == nil or y == 42 then");
        assert_eq!(find(&t, "nil"), Some(Tok::Literal));
        assert_eq!(find(&t, "42"), Some(Tok::Literal));
        assert_eq!(find(&t, "if"), Some(Tok::Keyword));
    }

    /// A keyword inside a string is not a keyword. This is the whole reason
    /// this is a tokenizer rather than a list of words to colour.
    #[test]
    fn a_keyword_inside_a_string_stays_a_string() {
        let t = kinds(r#"player:send("if you end this then")"#);
        let quoted = t.iter().find(|(_, k)| *k == Tok::Str).expect("a string run");
        assert!(quoted.0.contains("if you end this then"), "{quoted:?}");
        assert_eq!(find(&t, "if"), None, "`if` must not be a token of its own");
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_string() {
        let t = kinds(r#"local s = "a \" b" .. c"#);
        let s = t.iter().find(|(_, k)| *k == Tok::Str).unwrap();
        assert_eq!(s.0, r#""a \" b""#);
        assert_eq!(within(&t, ".. c"), Some(Tok::Plain), "code after it is code again");
    }

    #[test]
    fn a_comment_runs_to_the_end_of_the_line() {
        let t = kinds("local x = 1 -- set x, and \"quote\" nothing");
        let c = t.iter().find(|(_, k)| *k == Tok::Comment).unwrap();
        assert!(c.0.starts_with("-- set x"), "{c:?}");
        assert_eq!(
            t.iter().filter(|(_, k)| *k == Tok::Str).count(),
            0,
            "a quote inside a comment is not a string"
        );
    }

    /// The construct that cannot be decided one line at a time.
    #[test]
    fn a_long_string_spans_lines() {
        let src: Vec<String> = vec![
            "local d = [[".into(),
            "a musty workshop -- not a comment".into(),
            "]]".into(),
            "local y = 2".into(),
        ];
        let state = block_state(&src);
        assert_eq!(state, vec![None, Some(0), Some(0), None]);

        // The middle line is entirely inside the bracket.
        let t = tokenize(&src[1], state[1]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].1, Tok::Comment);

        // And the line after the close is code again.
        let t = tokenize(&src[3], state[3]);
        assert_eq!(find(&t, "local"), Some(Tok::Keyword));
    }

    #[test]
    fn a_long_comment_with_levels_needs_a_matching_close() {
        let src: Vec<String> = vec![
            "--[==[ start".into(),
            "]] not the end".into(),
            "]==] done".into(),
            "local z = 1".into(),
        ];
        let state = block_state(&src);
        assert_eq!(
            state,
            vec![None, Some(2), Some(2), None],
            "a `]]` must not close a `[==[`"
        );
    }

    /// Every line of the shipped mudlib tokenizes without panicking and without
    /// losing a character. Cheap, and it is the only way to be sure about a
    /// hand-rolled scanner.
    ///
    /// `tests/fixture/mudlib/` rather than `mudlib/`: the live tree is gitignored and
    /// absent on a clean clone, where `walk` would return nothing and the whole
    /// test would reduce to the `checked > 1000` guard below — which is exactly
    /// what that guard is for.
    #[test]
    fn every_line_of_the_mudlib_round_trips() {
        let mut checked = 0;
        for entry in walk("tests/fixture/mudlib") {
            let Ok(text) = std::fs::read_to_string(&entry) else { continue };
            let lines: Vec<String> = text.lines().map(str::to_string).collect();
            let state = block_state(&lines);
            for (i, line) in lines.iter().enumerate() {
                let joined: String = tokenize(line, state[i]).into_iter().map(|(t, _)| t).collect();
                assert_eq!(&joined, line, "{}:{} lost text", entry.display(), i + 1);
                checked += 1;
            }
        }
        assert!(checked > 1000, "expected a real corpus, saw {checked} lines");
    }

    fn walk(dir: &str) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(dir)];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "lua") {
                    out.push(p);
                }
            }
        }
        out
    }
}
