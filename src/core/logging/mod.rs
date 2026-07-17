//! Game-level logging subsystem.
//!
//! Provides [`GameLogger`] — a thread-safe, append-only structured log writer
//! for the audit trail (`logs/audit.log`) and the general server journal
//! (`logs/journal.log`).

pub mod game_logger;
pub use game_logger::{GameLogger, AuditEntry, JournalEntry, utc_now};
