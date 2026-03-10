//! Drain logic for per-CPU ring buffers.
//!
//! The drain function reads committed entries from all CPU ring buffers,
//! extracts formatted messages, and dispatches [`FormattedRecord`]s to all
//! registered sinks.

use crate::buffer::RINGS;
use crate::record::{MAX_FMT_BUF, RecordMessage};
use crate::sink::{FormattedRecord, log_sinks};

/// Synchronously drains all per-CPU ring buffers and dispatches to sinks.
///
/// Iterates all CPU slots, reads committed entries, and writes to every
/// registered sink. Called explicitly via [`flush()`](crate::flush) or
/// from the panic handler.
pub(crate) fn drain_all() {
    let sinks = log_sinks();

    for cpu_id in 0..hadron_core::cpu_local::MAX_CPUS {
        let ring = RINGS.get_for(cpu_id as u32);
        while let Some(record) = ring.pop() {
            let msg_str = match &record.message {
                RecordMessage::Formatted { buf, len } => {
                    let n = *len as usize;
                    // SAFETY: The buffer was written by core::fmt which
                    // guarantees valid UTF-8.
                    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
                }
            };

            let formatted = FormattedRecord {
                timestamp: record.timestamp,
                level: record.level,
                subsystem: record.subsystem,
                spans: &record.spans,
                message: msg_str,
                file: record.file,
                line: record.line,
            };

            for sink in sinks {
                if record.level <= sink.min_level {
                    (sink.write)(&formatted);
                }
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
