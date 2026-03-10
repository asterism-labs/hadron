//! Drain logic for the global MPSC ring buffer.
//!
//! The drain function reads published entries from the ring, formats each
//! into a complete log line, and dispatches [`FormattedRecord`]s to all
//! registered sinks.

use crate::record::{MAX_FMT_BUF, RecordMessage};
use crate::ring;
use crate::sink::{FormattedRecord, log_sinks};

/// Synchronously drains the global MPSC ring and dispatches to sinks.
///
/// Pops all published entries, formats each into a complete log line,
/// and writes to every registered sink whose `min_level` passes.
/// Called via [`flush()`](crate::flush), from auto-flush in `__emit_log`,
/// or from the panic handler.
pub(crate) fn drain_all() {
    let sinks = log_sinks();

    while let Some(record) = ring::RING.pop() {
        let msg_str = match &record.message {
            RecordMessage::Formatted { buf, len } => {
                let n = *len as usize;
                // SAFETY: The buffer was written by core::fmt which
                // guarantees valid UTF-8.
                unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
            }
        };

        let mut line_buf = [0u8; MAX_FMT_BUF];
        let line_len = format_full_line(
            &mut line_buf,
            record.level,
            record.subsystem,
            &record.spans,
            msg_str,
        );

        let formatted = FormattedRecord {
            timestamp: record.timestamp,
            level: record.level,
            subsystem: record.subsystem,
            spans: &record.spans,
            message: msg_str,
            file: record.file,
            line: record.line,
            formatted_line: &line_buf[..line_len],
        };

        for sink in sinks {
            if record.level <= sink.min_level {
                (sink.write)(&formatted);
            }
        }
    }

    for sink in sinks {
        (sink.flush)();
    }
}

/// Formats a log line prefix + message into a buffer.
///
/// Returns the number of bytes written. Format:
/// `[LEVEL subsystem] {spans} message\n`
pub(crate) fn format_full_line(
    buf: &mut [u8; MAX_FMT_BUF],
    level: crate::Level,
    subsystem: &str,
    spans: &crate::span::SpanSnapshot,
    message: &str,
) -> usize {
    let mut pos = crate::fmt::format_prefix(buf, level, subsystem, spans);

    let remaining = MAX_FMT_BUF.saturating_sub(pos);
    let msg_bytes = message.as_bytes();
    let n = msg_bytes.len().min(remaining);
    buf[pos..pos + n].copy_from_slice(&msg_bytes[..n]);
    pos += n;

    if pos < MAX_FMT_BUF {
        buf[pos] = b'\n';
        pos += 1;
    }

    pos
}
