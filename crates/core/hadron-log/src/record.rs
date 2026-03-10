//! Log record types.
//!
//! A [`LogRecord`] captures all information about a single log event.
//! The message is pre-formatted at the call site into a fixed 128-byte
//! inline buffer using `core::fmt`.

use crate::level::Level;
use crate::span::SpanSnapshot;

/// Maximum size of the pre-formatted message buffer in bytes.
pub const MAX_FMT_BUF: usize = 128;

/// A single log event, stored in per-CPU ring buffers.
pub struct LogRecord {
    /// Timestamp from `rdtsc` (or 0 during early boot).
    pub timestamp: u64,
    /// Severity level.
    pub level: Level,
    /// Subsystem that produced this message (e.g. `"acpi"`, `"smp"`).
    pub subsystem: &'static str,
    /// Source file path (from `file!()`).
    pub file: &'static str,
    /// Source line number (from `line!()`).
    pub line: u32,
    /// Snapshot of the span stack at log time.
    pub spans: SpanSnapshot,
    /// The log message.
    pub message: RecordMessage,
}

/// The message payload of a log record.
///
/// Formatted at the call site into a fixed-size inline buffer.
pub enum RecordMessage {
    /// Pre-formatted message using `core::fmt`.
    Formatted {
        /// UTF-8 formatted message bytes.
        buf: [u8; MAX_FMT_BUF],
        /// Number of valid bytes in `buf`.
        len: u8,
    },
}
