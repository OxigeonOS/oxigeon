//! `GameLogger` — append-only, thread-safe structured log writer.
//!
//! Writes two JSON-lines files:
//! - `logs/audit.log`  — privileged command audit trail (success + denial)
//! - `logs/journal.log` — general server journal (info/warn/error)
//!
//! Both files are created on first write. Entries are one JSON object per line
//! so they're `grep`-able and can be tailed with standard tools.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use crate::core::lock::MutexExt;

use serde_json::json;

/// An entry written to the audit log.
#[derive(Debug)]
pub struct AuditEntry<'a> {
    pub session_id:  &'a str,
    pub char_name:   &'a str,   // human-readable; empty string if unknown
    pub action:      &'a str,   // e.g. "efun.reload", "cmd.spawn"
    pub success:     bool,
    pub reason:      Option<&'a str>,
    pub extra:       Option<serde_json::Value>,
}

/// An entry written to the journal log.
#[derive(Debug)]
pub struct JournalEntry<'a> {
    pub level:   &'a str,  // "trace" | "debug" | "info" | "warn" | "error"
    pub source:  &'a str,  // e.g. "login.lua:42" or "driver"
    pub message: &'a str,
    pub meta:    Option<serde_json::Value>,
}

/// Thread-safe append-only log file.
struct LogFile {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl LogFile {
    fn new(path: PathBuf) -> Self {
        LogFile { path, file: Mutex::new(None) }
    }

    /// Open the file lazily (creates it and parent dirs on first write).
    fn write_line(&self, line: &str) {
        let mut guard = self.file.lock_recover();
        if guard.is_none() {
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
            {
                Ok(f) => *guard = Some(f),
                Err(e) => {
                    tracing::error!("Failed to open log file {:?}: {}", self.path, e);
                    return;
                }
            }
        }
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{}", line);
        }
    }

    /// Read the last `limit` lines of the file. Returns them oldest-first.
    pub fn read_tail(&self, limit: usize) -> Vec<String> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => {
                let lines: Vec<String> = content
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.to_string())
                    .collect();
                let start = lines.len().saturating_sub(limit);
                lines[start..].to_vec()
            }
            Err(_) => vec![],
        }
    }

    /// Read lines filtered to those containing `level_substr` (case-insensitive).
    pub fn read_tail_filtered(&self, limit: usize, level_filter: &str) -> Vec<String> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => {
                let filter = level_filter.to_lowercase();
                let matching: Vec<String> = content
                    .lines()
                    .filter(|l| !l.trim().is_empty() && l.to_lowercase().contains(&filter))
                    .map(|l| l.to_string())
                    .collect();
                let start = matching.len().saturating_sub(limit);
                matching[start..].to_vec()
            }
            Err(_) => vec![],
        }
    }
}

/// The central game logger. Clone the `Arc<GameLogger>` freely — all writes
/// go through internal `Mutex`-protected file handles.
pub struct GameLogger {
    audit:   LogFile,
    journal: LogFile,
}

impl GameLogger {
    /// Create a new `GameLogger` writing to the given directory.
    /// Files are NOT opened until the first write.
    pub fn new(log_dir: &Path) -> Self {
        GameLogger {
            audit:   LogFile::new(log_dir.join("audit.log")),
            journal: LogFile::new(log_dir.join("journal.log")),
        }
    }

    // ─── Audit log ──────────────────────────────────────────────────────────

    /// Write an audit entry.
    pub fn audit(&self, entry: AuditEntry<'_>) {
        let ts = utc_now();
        let mut obj = json!({
            "ts":      ts,
            "sid":     entry.session_id,
            "char":    entry.char_name,
            "action":  entry.action,
            "success": entry.success,
            "reason":  entry.reason,
        });
        if let Some(extra) = entry.extra {
            if let (serde_json::Value::Object(base), serde_json::Value::Object(ext)) =
                (&mut obj, extra)
            {
                base.extend(ext);
            }
        }
        self.audit.write_line(&obj.to_string());
    }

    /// Read the last `limit` audit lines.
    pub fn read_audit(&self, limit: usize) -> Vec<String> {
        self.audit.read_tail(limit)
    }

    // ─── Journal log ────────────────────────────────────────────────────────

    /// Write a journal entry.
    pub fn journal(&self, entry: JournalEntry<'_>) {
        let ts = utc_now();
        let obj = json!({
            "ts":     ts,
            "level":  entry.level,
            "source": entry.source,
            "msg":    entry.message,
            "meta":   entry.meta,
        });
        self.journal.write_line(&obj.to_string());
    }

    /// Read the last `limit` journal lines, optionally filtered by level string.
    pub fn read_journal(&self, limit: usize, level_filter: Option<&str>) -> Vec<String> {
        match level_filter {
            None => self.journal.read_tail(limit),
            Some(f) => self.journal.read_tail_filtered(limit, f),
        }
    }
}

/// Current UTC timestamp in RFC 3339 format without pulling in `chrono`.
pub fn utc_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format as a simple ISO 8601 UTC string
    let s = secs % 86400;
    let m = s / 60;
    let h = m / 60;
    let days = secs / 86400;
    // Days since Unix epoch → calendar date (simple Gregorian approximation)
    let (year, month, day) = epoch_days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day,
        h, m % 60, s % 60
    )
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
/// Standard algorithm; handles leap years correctly.
fn epoch_days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Civil date from days since epoch — standard algorithm
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_logger() -> (GameLogger, TempDir) {
        let dir = TempDir::new().unwrap();
        let logger = GameLogger::new(dir.path());
        (logger, dir)
    }

    #[test]
    fn test_audit_write_and_read() {
        let (logger, _dir) = make_logger();
        logger.audit(AuditEntry {
            session_id: "abc123",
            char_name:  "Gandalf",
            action:     "efun.reload",
            success:    true,
            reason:     None,
            extra:      None,
        });
        let lines = logger.read_audit(10);
        assert_eq!(lines.len(), 1);
        let obj: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(obj["action"], "efun.reload");
        assert_eq!(obj["char"], "Gandalf");
        assert_eq!(obj["success"], true);
    }

    #[test]
    fn test_audit_denial_entry() {
        let (logger, _dir) = make_logger();
        logger.audit(AuditEntry {
            session_id: "xyz",
            char_name:  "Sauron",
            action:     "efun.broadcast",
            success:    false,
            reason:     Some("missing permission efun.broadcast"),
            extra:      None,
        });
        let lines = logger.read_audit(10);
        let obj: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(obj["success"], false);
        assert_eq!(obj["reason"], "missing permission efun.broadcast");
    }

    #[test]
    fn test_journal_write_and_read() {
        let (logger, _dir) = make_logger();
        logger.journal(JournalEntry {
            level:   "error",
            source:  "login.lua:42",
            message: "attempt to index nil",
            meta:    Some(serde_json::json!({"sid": "abc", "char": "Alice"})),
        });
        let lines = logger.read_journal(10, None);
        assert_eq!(lines.len(), 1);
        let obj: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(obj["level"], "error");
        assert_eq!(obj["source"], "login.lua:42");
    }

    #[test]
    fn test_journal_level_filter() {
        let (logger, _dir) = make_logger();
        logger.journal(JournalEntry { level: "info",  source: "driver", message: "started", meta: None });
        logger.journal(JournalEntry { level: "error", source: "foo.lua:1", message: "oops", meta: None });
        logger.journal(JournalEntry { level: "warn",  source: "bar.lua:2", message: "hmm", meta: None });

        let errors = logger.read_journal(10, Some("error"));
        assert_eq!(errors.len(), 1);
        let obj: serde_json::Value = serde_json::from_str(&errors[0]).unwrap();
        assert_eq!(obj["level"], "error");
    }

    #[test]
    fn test_read_tail_limit() {
        let (logger, _dir) = make_logger();
        for i in 0..20 {
            logger.audit(AuditEntry {
                session_id: "s",
                char_name:  "X",
                action:     &format!("cmd.{}", i),
                success:    true,
                reason:     None,
                extra:      None,
            });
        }
        let lines = logger.read_audit(5);
        assert_eq!(lines.len(), 5);
        // Last 5 should be cmd.15 through cmd.19
        let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert_eq!(last["action"], "cmd.19");
    }

    #[test]
    fn test_missing_log_dir_created_on_write() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        let logger = GameLogger::new(&nested);
        // Should NOT panic; should create dirs and write
        logger.journal(JournalEntry {
            level: "info", source: "test", message: "hello", meta: None,
        });
        let lines = logger.read_journal(5, None);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_empty_file_returns_empty_vec() {
        let (logger, _dir) = make_logger();
        // Nothing written yet
        assert!(logger.read_audit(10).is_empty());
        assert!(logger.read_journal(10, None).is_empty());
    }

    #[test]
    fn test_utc_now_format() {
        let ts = utc_now();
        // Should match YYYY-MM-DDTHH:MM:SSZ
        assert_eq!(ts.len(), 20);
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }

    #[test]
    fn test_concurrent_writes_dont_corrupt() {
        use std::sync::Arc;
        use std::thread;

        let dir = TempDir::new().unwrap();
        let logger = Arc::new(GameLogger::new(dir.path()));

        let handles: Vec<_> = (0..8).map(|i| {
            let l = logger.clone();
            thread::spawn(move || {
                for j in 0..10 {
                    l.audit(AuditEntry {
                        session_id: "s",
                        char_name:  "X",
                        action:     &format!("cmd.{}.{}", i, j),
                        success:    true,
                        reason:     None,
                        extra:      None,
                    });
                }
            })
        }).collect();

        for h in handles { h.join().unwrap(); }

        let lines = logger.read_audit(100);
        assert_eq!(lines.len(), 80);
        // Every line should parse as valid JSON
        for line in &lines {
            serde_json::from_str::<serde_json::Value>(line)
                .expect("corrupted JSON line");
        }
    }
}
