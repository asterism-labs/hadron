//! Log output formatting helpers.
//!
//! Provides prefix formatting for log lines. The actual message content
//! is formatted at the call site using `core::fmt`.

use crate::record::MAX_FMT_BUF;

/// Writes raw bytes into the buffer, truncating if necessary.
fn write_bytes(buf: &mut [u8; MAX_FMT_BUF], pos: usize, bytes: &[u8]) -> usize {
    let remaining = MAX_FMT_BUF.saturating_sub(pos);
    let n = bytes.len().min(remaining);
    buf[pos..pos + n].copy_from_slice(&bytes[..n]);
    pos + n
}

/// Formats a log line prefix: `[LEVEL subsystem] {spans} `
///
/// Used by both the drain path and early serial output.
pub(crate) fn format_prefix(
    buf: &mut [u8; MAX_FMT_BUF],
    level: crate::Level,
    subsystem: &str,
    spans: &crate::span::SpanSnapshot,
) -> usize {
    let mut pos = 0;
    pos = write_bytes(buf, pos, b"[");

    let level_str = match level {
        crate::Level::FATAL => "FATAL",
        crate::Level::ERROR => "ERROR",
        crate::Level::WARN => " WARN",
        crate::Level::INFO => " INFO",
        crate::Level::DEBUG => "DEBUG",
        crate::Level::TRACE => "TRACE",
        _ => "?????",
    };
    pos = write_bytes(buf, pos, level_str.as_bytes());
    pos = write_bytes(buf, pos, b" ");
    pos = write_bytes(buf, pos, subsystem.as_bytes());
    pos = write_bytes(buf, pos, b"]");

    // Append span labels: " {span1>span2>...}"
    if spans.depth > 0 {
        pos = write_bytes(buf, pos, b" {");
        for (i, label) in spans.iter().enumerate() {
            if i > 0 {
                pos = write_bytes(buf, pos, b">");
            }
            pos = write_bytes(buf, pos, label.as_bytes());
        }
        pos = write_bytes(buf, pos, b"}");
    }

    pos = write_bytes(buf, pos, b" ");
    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::SpanSnapshot;

    #[test]
    fn prefix_info_no_spans() {
        let mut buf = [0u8; MAX_FMT_BUF];
        let spans = SpanSnapshot::empty();
        let len = format_prefix(&mut buf, crate::Level::INFO, "acpi", &spans);
        assert_eq!(core::str::from_utf8(&buf[..len]).unwrap(), "[ INFO acpi] ");
    }

    #[test]
    fn prefix_with_spans() {
        let mut buf = [0u8; MAX_FMT_BUF];
        let spans = SpanSnapshot {
            labels: [
                Some("init"),
                Some("pci"),
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            depth: 2,
        };
        let len = format_prefix(&mut buf, crate::Level::DEBUG, "mm", &spans);
        assert_eq!(
            core::str::from_utf8(&buf[..len]).unwrap(),
            "[DEBUG mm] {init>pci} "
        );
    }
}
