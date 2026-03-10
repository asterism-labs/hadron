//! Log output sinks registered via linker sections.
//!
//! A [`LogSink`] is a static struct placed in the `hadron_log_sinks` linker
//! section. The drain task dispatches formatted records to all registered
//! sinks whose `min_level` passes.

use crate::level::Level;
use crate::span::SpanSnapshot;

/// A log output backend, registered at link time via [`linkset_entry!`].
pub struct LogSink {
    /// Human-readable sink name (e.g. `"serial"`, `"framebuffer"`).
    pub name: &'static str,
    /// Write a formatted log record to this sink.
    pub write: fn(&FormattedRecord<'_>),
    /// Flush any buffered output. Called during panic or explicit flush.
    pub flush: fn(),
    /// Minimum severity level this sink accepts. Messages with a numeric
    /// level greater than this are skipped.
    pub min_level: Level,
}

/// A fully formatted log record ready for output to sinks.
pub struct FormattedRecord<'a> {
    /// Timestamp (rdtsc value).
    pub timestamp: u64,
    /// Severity level.
    pub level: Level,
    /// Subsystem identifier.
    pub subsystem: &'static str,
    /// Span chain snapshot.
    pub spans: &'a SpanSnapshot,
    /// The formatted message string.
    pub message: &'a str,
    /// Source file path.
    pub file: &'static str,
    /// Source line number.
    pub line: u32,
}

// ── Linkset declaration ─────────────────────────────────────────────────

hadron_linkset::declare_linkset! {
    /// Returns all registered log sinks from the `hadron_log_sinks` linker section.
    pub fn log_sinks() -> [LogSink],
    section = "hadron_log_sinks"
}
