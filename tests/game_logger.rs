//! Integration tests for the public API of GameLogger.
//! These test the logger from the crate's public re-exports, not the internal module.

use std::sync::Arc;
use oxigeon::core::logging::{GameLogger, AuditEntry, JournalEntry};
use tempfile::TempDir;

fn make() -> (GameLogger, TempDir) {
    let dir = TempDir::new().unwrap();
    let logger = GameLogger::new(dir.path());
    (logger, dir)
}

#[test]
fn test_public_api_audit_roundtrip() {
    let (logger, _dir) = make();
    logger.audit(AuditEntry {
        session_id: "test-session-123",
        char_name:  "Thorin",
        action:     "efun.reload",
        success:    true,
        reason:     None,
        extra:      None,
    });
    let lines = logger.read_audit(10);
    assert_eq!(lines.len(), 1, "Expected exactly one audit entry");
    let obj: serde_json::Value = serde_json::from_str(&lines[0])
        .expect("Audit entry must be valid JSON");
    assert_eq!(obj["action"], "efun.reload");
    assert_eq!(obj["char"], "Thorin");
    assert_eq!(obj["sid"], "test-session-123");
    assert_eq!(obj["success"], true);
    assert!(obj["ts"].is_string(), "Timestamp must be a string");
}

#[test]
fn test_public_api_journal_roundtrip() {
    let (logger, _dir) = make();
    logger.journal(JournalEntry {
        level:   "warn",
        source:  "startup.lua:5",
        message: "mudlib version mismatch",
        meta:    Some(serde_json::json!({"expected": "2.0", "found": "1.9"})),
    });
    let lines = logger.read_journal(10, None);
    assert_eq!(lines.len(), 1, "Expected exactly one journal entry");
    let obj: serde_json::Value = serde_json::from_str(&lines[0])
        .expect("Journal entry must be valid JSON");
    assert_eq!(obj["level"], "warn");
    assert_eq!(obj["source"], "startup.lua:5");
    assert_eq!(obj["msg"], "mudlib version mismatch");
    assert_eq!(obj["meta"]["expected"], "2.0");
}

#[test]
fn test_public_api_audit_failure_with_reason() {
    let (logger, _dir) = make();
    logger.audit(AuditEntry {
        session_id: "bad-session",
        char_name:  "",
        action:     "audit_read",
        success:    false,
        reason:     Some("permission denied"),
        extra:      Some(serde_json::json!({"required": "admin"})),
    });
    let lines = logger.read_audit(10);
    assert_eq!(lines.len(), 1);
    let obj: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(obj["success"], false);
    assert_eq!(obj["reason"], "permission denied");
    assert_eq!(obj["required"], "admin", "Extra fields should be merged at top level");
}

#[test]
fn test_public_api_journal_level_filter() {
    let (logger, _dir) = make();
    logger.journal(JournalEntry { level: "info",  source: "driver", message: "started",    meta: None });
    logger.journal(JournalEntry { level: "error", source: "lua",    message: "nil access", meta: None });
    logger.journal(JournalEntry { level: "warn",  source: "driver", message: "slow tick",  meta: None });
    logger.journal(JournalEntry { level: "error", source: "lua",    message: "stack overflow", meta: None });

    let errors = logger.read_journal(100, Some("error"));
    assert_eq!(errors.len(), 2, "Should have exactly 2 error entries");
    for line in &errors {
        let obj: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(obj["level"], "error");
    }

    let warnings = logger.read_journal(100, Some("warn"));
    assert_eq!(warnings.len(), 1);

    let all = logger.read_journal(100, None);
    assert_eq!(all.len(), 4);
}

#[test]
fn test_public_api_read_tail_respects_limit() {
    let (logger, _dir) = make();
    for i in 0..30 {
        logger.audit(AuditEntry {
            session_id: "s",
            char_name:  "X",
            action:     &format!("cmd.{}", i),
            success:    true,
            reason:     None,
            extra:      None,
        });
    }
    let lines = logger.read_audit(10);
    assert_eq!(lines.len(), 10);
    // Last entry should be cmd.29
    let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert_eq!(last["action"], "cmd.29");
    // First in tail should be cmd.20
    let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(first["action"], "cmd.20");
}

#[test]
fn test_public_api_concurrent_thread_safety() {
    let dir = TempDir::new().unwrap();
    let logger = Arc::new(GameLogger::new(dir.path()));

    let handles: Vec<_> = (0..6).map(|i| {
        let l = logger.clone();
        std::thread::spawn(move || {
            for j in 0..15 {
                l.journal(JournalEntry {
                    level:   "info",
                    source:  "test",
                    message: &format!("thread {} msg {}", i, j),
                    meta:    None,
                });
            }
        })
    }).collect();

    for h in handles { h.join().unwrap(); }

    let lines = logger.read_journal(200, None);
    assert_eq!(lines.len(), 90, "Expected 6 * 15 = 90 journal lines");
    for line in &lines {
        serde_json::from_str::<serde_json::Value>(line)
            .expect("Each line must be valid JSON even under concurrent writes");
    }
}

#[test]
fn test_public_api_empty_logger_returns_empty() {
    let (logger, _dir) = make();
    assert!(logger.read_audit(10).is_empty());
    assert!(logger.read_journal(10, None).is_empty());
    assert!(logger.read_journal(10, Some("error")).is_empty());
}

#[test]
fn test_public_api_journal_metadata_preserved() {
    let (logger, _dir) = make();
    let meta = serde_json::json!({
        "sid": "abc-123",
        "char": "Legolas",
        "input_len": 42,
    });
    logger.journal(JournalEntry {
        level:   "info",
        source:  "on_input",
        message: "player typed command",
        meta:    Some(meta),
    });
    let lines = logger.read_journal(5, None);
    assert_eq!(lines.len(), 1);
    let obj: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(obj["meta"]["sid"], "abc-123");
    assert_eq!(obj["meta"]["char"], "Legolas");
    assert_eq!(obj["meta"]["input_len"], 42);
}
