//! Trace ring buffers and their rendering.
//!
//! Both rings live in a thread-local on the Lua thread. The writer (the hook)
//! and the reader (the `trace_*` efuns, which are only ever called from Lua)
//! are the same thread, so no synchronization is needed at all — the same
//! reasoning behind `CURRENT_SESSION` in [`super::super::efuns`].

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

use super::paths;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TraceKind {
    Call,
    Ret,
    Line,
}

impl TraceKind {
    fn glyph(self) -> char {
        match self {
            Self::Call => '>',
            Self::Ret => '<',
            Self::Line => ' ',
        }
    }
}

/// One recorded hook event.
///
/// `src` and `name` are interned `Arc<str>` rather than `String`: a fresh
/// allocation per traced line is the one thing that would make line mode
/// unusable on a busy MUD.
pub struct TraceRecord {
    pub kind: TraceKind,
    pub depth: u16,
    pub src: Arc<str>,
    pub line: u32,
    pub name: Option<Arc<str>>,
    pub micros: u32,
}

/// One completed input dispatch.
pub struct CommandTiming {
    pub session: String,
    pub verb: String,
    pub micros: u64,
    pub lines: u32,
    pub calls: u32,
    pub max_depth: u16,
}

#[derive(Default)]
pub struct Rings {
    pub trace: VecDeque<TraceRecord>,
    pub timing: VecDeque<CommandTiming>,
    pub trace_cap: usize,
    pub timing_cap: usize,
    /// Records dropped because the ring was full, since the last clear.
    pub dropped: u64,
}

thread_local! {
    static RINGS: RefCell<Rings> = RefCell::new(Rings::default());
}

/// Run `f` against the Lua thread's rings.
pub fn with_rings<R>(f: impl FnOnce(&mut Rings) -> R) -> R {
    RINGS.with(|r| f(&mut r.borrow_mut()))
}

pub fn set_capacities(trace_cap: usize, timing_cap: usize) {
    with_rings(|r| {
        r.trace_cap = trace_cap;
        r.timing_cap = timing_cap;
        r.trace.truncate(trace_cap);
        r.timing.truncate(timing_cap);
    });
}

pub fn push_record(rec: TraceRecord) {
    with_rings(|r| {
        if r.trace_cap == 0 {
            return;
        }
        while r.trace.len() >= r.trace_cap {
            r.trace.pop_front();
            r.dropped += 1;
        }
        r.trace.push_back(rec);
    });
}

pub fn push_timing(t: CommandTiming) {
    with_rings(|r| {
        if r.timing_cap == 0 {
            return;
        }
        while r.timing.len() >= r.timing_cap {
            r.timing.pop_front();
        }
        r.timing.push_back(t);
    });
}

pub fn clear() {
    with_rings(|r| {
        r.trace.clear();
        r.timing.clear();
        r.dropped = 0;
    });
}

/// Render the most recent `limit` trace records, oldest first.
///
/// Plain text with no colour tags: paged output goes through
/// `DAEMON.pager.page`, which calls the raw `send()` efun and therefore skips
/// `Player:_process_output`'s colorize step — tags would render literally.
pub fn format_records(limit: usize) -> Vec<String> {
    with_rings(|r| {
        let skip = r.trace.len().saturating_sub(limit);
        r.trace
            .iter()
            .skip(skip)
            .map(|rec| {
                let indent = " ".repeat((rec.depth as usize).min(20));
                let name = rec.name.as_deref().unwrap_or("");
                // Line 0 means a C function, which has no source location.
                let site = if rec.line == 0 {
                    paths::short(&rec.src)
                } else {
                    format!("{}:{}", paths::short(&rec.src), rec.line)
                };
                format!(
                    "{:>8.3}ms {} {}{}{}{}",
                    rec.micros as f64 / 1000.0,
                    rec.kind.glyph(),
                    indent,
                    site,
                    if name.is_empty() { "" } else { "  " },
                    name,
                )
            })
            .collect()
    })
}

/// Render the most recent `limit` command timings, newest last.
pub fn format_timings(limit: usize) -> Vec<String> {
    with_rings(|r| {
        let skip = r.timing.len().saturating_sub(limit);
        let mut out = vec![format!(
            "{:>10}  {:<14} {:>8} {:>8} {:>6}",
            "elapsed", "verb", "lines", "calls", "depth"
        )];
        out.extend(r.timing.iter().skip(skip).map(|t| {
            format!(
                "{:>8.2}ms  {:<14} {:>8} {:>8} {:>6}",
                t.micros as f64 / 1000.0,
                truncate(&t.verb, 14),
                t.lines,
                t.calls,
                t.max_depth,
            )
        }));
        out
    })
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).chain(['…']).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(line: u32) -> TraceRecord {
        TraceRecord {
            kind: TraceKind::Line,
            depth: 1,
            src: Arc::from("@C:/x/mudlib/cmds/who.lua"),
            line,
            name: None,
            micros: 1500,
        }
    }

    #[test]
    fn ring_evicts_oldest_and_counts_drops() {
        clear();
        set_capacities(3, 2);
        for i in 1..=5 {
            push_record(rec(i));
        }
        with_rings(|r| {
            assert_eq!(r.trace.len(), 3);
            assert_eq!(r.dropped, 2);
            assert_eq!(r.trace.front().unwrap().line, 3, "oldest two evicted");
        });
        clear();
    }

    #[test]
    fn records_render_without_colour_tags_or_absolute_paths() {
        clear();
        set_capacities(8, 2);
        push_record(rec(42));
        let out = format_records(10);
        clear();

        assert_eq!(out.len(), 1);
        assert!(out[0].contains("cmds/who.lua:42"), "got {out:?}");
        assert!(!out[0].contains('{'), "paged body must be plain text: {out:?}");
        assert!(!out[0].contains("C:/x"), "path should be shortened: {out:?}");
    }

    #[test]
    fn zero_capacity_drops_everything_without_panicking() {
        clear();
        set_capacities(0, 0);
        push_record(rec(1));
        push_timing(CommandTiming {
            session: "1".into(),
            verb: "who".into(),
            micros: 10,
            lines: 1,
            calls: 1,
            max_depth: 1,
        });
        with_rings(|r| assert!(r.trace.is_empty() && r.timing.is_empty()));
        clear();
    }
}
