//! Early boot serial output (Phase 0).
//!
//! Before `CpuLocal` is initialized, log macros format synchronously and
//! write directly to COM1 (port `0x3F8`). This bypasses the ring buffer
//! and sink infrastructure entirely.

use crate::drain::format_full_line;
use crate::record::{MAX_FMT_BUF, RecordMessage};
use crate::span::SpanSnapshot;

/// COM1 data register port.
const COM1_PORT: u16 = 0x3F8;

/// Writes a byte to COM1 via `out dx, al`.
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
#[inline]
fn serial_byte(b: u8) {
    // SAFETY: Port 0x3F8 is the standard COM1 data register. Writing
    // bytes to it is safe during early boot (no contention).
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") COM1_PORT,
            in("al") b,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// No-op on non-x86_64 targets (host tests).
#[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
#[inline]
fn serial_byte(_b: u8) {}

/// Writes a byte slice to COM1.
fn serial_bytes(bytes: &[u8]) {
    for &b in bytes {
        serial_byte(b);
    }
}

/// Emits a log message synchronously to COM1 (Phase 0 path).
///
/// Called by the log macros when `cpu_is_initialized()` returns `false`.
/// Formats the prefix and writes the complete line to serial.
pub(crate) fn emit_serial_sync(
    level: crate::Level,
    subsystem: &'static str,
    message: &RecordMessage,
) {
    let spans = SpanSnapshot::empty();

    let msg_str = match message {
        RecordMessage::Formatted { buf, len } => {
            let n = *len as usize;
            // SAFETY: The buffer was written by core::fmt which guarantees UTF-8.
            unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
        }
    };

    let mut line_buf = [0u8; MAX_FMT_BUF];
    let line_len = format_full_line(&mut line_buf, level, subsystem, &spans, msg_str);

    serial_bytes(&line_buf[..line_len]);
}
