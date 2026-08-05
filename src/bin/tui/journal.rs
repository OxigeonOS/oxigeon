//! Tail of `logs/journal.log`.
//!
//! The driver writes one JSON object per line there (`src/core/logging/game_logger.rs`)
//! and captures *every* Lua error with its traceback, so this pane shows a
//! mudlib crash whether or not a debugger was attached when it happened. It
//! needs no cooperation from the server at all — it is a file.

use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;

/// How much history to show on startup.
const BACKFILL: usize = 200;

#[derive(Debug, Clone)]
pub struct Entry {
    pub ts: String,
    pub level: String,
    pub source: String,
    pub msg: String,
}

impl Entry {
    /// Parse one journal line. Anything unparseable is still shown — a
    /// half-written line during a crash is exactly when you want to see it.
    pub fn parse(line: &str) -> Self {
        let get = |v: &serde_json::Value, k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        };
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => Entry {
                ts: get(&v, "ts"),
                level: get(&v, "level"),
                source: get(&v, "source"),
                msg: get(&v, "msg"),
            },
            Err(_) => Entry {
                ts: String::new(),
                level: "raw".into(),
                source: String::new(),
                msg: line.to_string(),
            },
        }
    }

    /// `2026-08-03T18:31:02Z` → `18:31:02`. Timestamps are always 20 chars
    /// ending in `Z`, hand-formatted by the driver.
    pub fn clock(&self) -> &str {
        self.ts.get(11..19).unwrap_or(&self.ts)
    }

    pub fn matches(&self, needle: &str) -> bool {
        let needle = needle.to_ascii_lowercase();
        self.level.to_ascii_lowercase().contains(&needle)
            || self.source.to_ascii_lowercase().contains(&needle)
            || self.msg.to_ascii_lowercase().contains(&needle)
    }
}

pub async fn run(path: String, events: UnboundedSender<AppEvent>) {
    // Position after backfill, so the first poll does not replay the whole file.
    let mut offset = match backfill(&path, &events) {
        Some(end) => end,
        None => 0,
    };

    loop {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if events.is_closed() {
            return;
        }

        let Ok(meta) = std::fs::metadata(&path) else {
            // Not created yet — the driver opens it lazily on first write.
            continue;
        };
        let len = meta.len();
        if len < offset {
            // Truncated or rotated out from under us; start over.
            offset = 0;
        }
        if len == offset {
            continue;
        }

        let Ok(mut file) = std::fs::File::open(&path) else {
            continue;
        };
        if file.seek(SeekFrom::Start(offset)).is_err() {
            continue;
        }
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(n) => {
                    // A line without a terminator is still being written; leave
                    // the offset before it and pick it up whole next time.
                    if !line.ends_with('\n') {
                        break;
                    }
                    offset += n as u64;
                    let trimmed = line.trim_end();
                    if !trimmed.is_empty()
                        && events
                            .send(AppEvent::Journal(Entry::parse(trimmed)))
                            .is_err()
                    {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
    }
}

/// Emit the tail of the existing file and return the byte offset of its end.
fn backfill(path: &str, events: &UnboundedSender<AppEvent>) -> Option<u64> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(BACKFILL);
    for line in &lines[start..] {
        if events.send(AppEvent::Journal(Entry::parse(line))).is_err() {
            return None;
        }
    }
    Some(content.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_driver_written_entry() {
        let raw = r#"{"ts":"2026-07-15T18:30:00Z","level":"error","source":"login.lua:42","msg":"attempt to index a nil value","meta":{"sid":"f3a2b1c0"}}"#;
        let e = Entry::parse(raw);
        assert_eq!(e.level, "error");
        assert_eq!(e.source, "login.lua:42");
        assert_eq!(e.clock(), "18:30:00");
        assert_eq!(e.msg, "attempt to index a nil value");
    }

    #[test]
    fn an_unparseable_line_is_shown_rather_than_dropped() {
        let e = Entry::parse("{half written");
        assert_eq!(e.level, "raw");
        assert_eq!(e.msg, "{half written");
    }

    #[test]
    fn filtering_looks_at_level_source_and_message() {
        let e = Entry::parse(r#"{"ts":"","level":"warn","source":"mob_d.lua:9","msg":"template missing"}"#);
        assert!(e.matches("WARN"));
        assert!(e.matches("mob_d"));
        assert!(e.matches("missing"));
        assert!(!e.matches("combat"));
    }
}
