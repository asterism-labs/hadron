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

/// Writes a decimal integer right-aligned in a fixed-width field.
///
/// Returns the updated position.
fn write_decimal(buf: &mut [u8; MAX_FMT_BUF], pos: usize, value: u64, width: usize) -> usize {
    let mut tmp = [b' '; 16];
    let mut v = value;
    let mut i = tmp.len();

    // Generate digits in reverse.
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }

    let digits = tmp.len() - i;
    let pad = width.saturating_sub(digits);

    // Write leading spaces.
    let mut p = pos;
    for _ in 0..pad {
        p = write_bytes(buf, p, b" ");
    }
    // Write digits.
    write_bytes(buf, p, &tmp[i..])
}

/// Writes a decimal integer zero-padded to exactly `width` digits.
fn write_decimal_zero(buf: &mut [u8; MAX_FMT_BUF], pos: usize, value: u64, width: usize) -> usize {
    let mut tmp = [b'0'; 16];
    let mut v = value;
    let mut i = tmp.len();

    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }

    let digits = tmp.len() - i;
    // If fewer digits than width, the leading zeros in tmp[..i] handle it.
    if digits >= width {
        write_bytes(buf, pos, &tmp[tmp.len() - width..])
    } else {
        // Write leading zeros then digits.
        let pad = width - digits;
        let mut p = pos;
        for _ in 0..pad {
            p = write_bytes(buf, p, b"0");
        }
        write_bytes(buf, p, &tmp[i..])
    }
}

/// Writes a hex integer zero-padded to exactly `width` nibbles.
fn write_hex(buf: &mut [u8; MAX_FMT_BUF], pos: usize, value: u64, width: usize) -> usize {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut tmp = [b'0'; 16];
    let mut v = value;
    let mut i = tmp.len();

    loop {
        i -= 1;
        tmp[i] = HEX[(v & 0xF) as usize];
        v >>= 4;
        if v == 0 {
            break;
        }
    }

    let nibbles = tmp.len() - i;
    if nibbles >= width {
        write_bytes(buf, pos, &tmp[tmp.len() - width..])
    } else {
        let pad = width - nibbles;
        let mut p = pos;
        for _ in 0..pad {
            p = write_bytes(buf, p, b"0");
        }
        write_bytes(buf, p, &tmp[i..])
    }
}

/// Formats a log line prefix with timestamp.
///
/// With calibrated nanoseconds (`tsc_nanos = Some(ns)`):
/// `[  X.YYYYYYs LEVEL sub] `
///
/// Before calibration (`tsc_nanos = None`):
/// `[tsc:XXXXXXXXXX LEVEL sub] `
pub(crate) fn format_prefix(
    buf: &mut [u8; MAX_FMT_BUF],
    timestamp: u64,
    tsc_nanos: Option<u64>,
    level: crate::Level,
    subsystem: &str,
    spans: &crate::span::SpanSnapshot,
) -> usize {
    let mut pos = 0;
    pos = write_bytes(buf, pos, b"[");

    if let Some(ns) = tsc_nanos {
        let secs = ns / 1_000_000_000;
        let frac = (ns % 1_000_000_000) / 1_000; // microseconds
        pos = write_decimal(buf, pos, secs, 3);
        pos = write_bytes(buf, pos, b".");
        pos = write_decimal_zero(buf, pos, frac, 6);
        pos = write_bytes(buf, pos, b"s ");
    } else {
        pos = write_bytes(buf, pos, b"tsc:");
        pos = write_hex(buf, pos, timestamp, 10);
        pos = write_bytes(buf, pos, b" ");
    }

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
    fn prefix_with_nanos() {
        let mut buf = [0u8; MAX_FMT_BUF];
        let spans = SpanSnapshot::empty();
        let len = format_prefix(
            &mut buf,
            0,
            Some(1_847_000), // 0.001847s
            crate::Level::INFO,
            "boot",
            &spans,
        );
        assert_eq!(
            core::str::from_utf8(&buf[..len]).unwrap(),
            "[  0.001847s  INFO boot] "
        );
    }

    #[test]
    fn prefix_raw_tsc() {
        let mut buf = [0u8; MAX_FMT_BUF];
        let spans = SpanSnapshot::empty();
        let len = format_prefix(
            &mut buf,
            0x0000_003A_F2C1,
            None,
            crate::Level::INFO,
            "boot",
            &spans,
        );
        assert_eq!(
            core::str::from_utf8(&buf[..len]).unwrap(),
            "[tsc:00003AF2C1  INFO boot] "
        );
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
        let len = format_prefix(
            &mut buf,
            0,
            Some(500_000_000), // 0.500000s
            crate::Level::DEBUG,
            "mm",
            &spans,
        );
        assert_eq!(
            core::str::from_utf8(&buf[..len]).unwrap(),
            "[  0.500000s DEBUG mm] {init>pci} "
        );
    }

    #[test]
    fn prefix_large_timestamp() {
        let mut buf = [0u8; MAX_FMT_BUF];
        let spans = SpanSnapshot::empty();
        // 123.456789 seconds = 123_456_789_000 nanos
        let len = format_prefix(
            &mut buf,
            0,
            Some(123_456_789_000),
            crate::Level::WARN,
            "net",
            &spans,
        );
        assert_eq!(
            core::str::from_utf8(&buf[..len]).unwrap(),
            "[123.456789s  WARN net] "
        );
    }
}
