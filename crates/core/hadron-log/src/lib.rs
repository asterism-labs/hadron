//! Structured logging subsystem for the Hadron kernel.
//!
//! Provides severity-based logging with subsystem tagging, per-CPU ring
//! buffering, span-scoped context, and linkset-based sink registration.
//! All messages are formatted at the call site using `core::fmt` into a
//! fixed 128-byte inline buffer.
//!
//! # Phases
//!
//! - **Phase 0** (early boot): Before `CpuLocal` is initialized, log entries
//!   are formatted synchronously and written directly to COM1.
//! - **Phase 1**: After per-CPU data is live, entries are buffered in a small
//!   static per-CPU ring and drained synchronously on `flush()`.
//!
//! # Usage
//!
//! ```ignore
//! use hadron_log::{kinfo, kdebug, kwarn, kerror, kspan};
//!
//! kinfo!("acpi", "found {} DRHD entries", count);
//! kwarn!("mm", "page fault at {:#x}", addr);
//!
//! let _guard = kspan!("irq_handler");
//! kdebug!("irq", "handling vector {}", vector);
//! ```

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

mod buffer;
mod drain;
mod early;
mod fmt;
mod level;
mod macros;
mod record;
mod sink;
mod span;

pub use level::{Level, get_runtime_level, runtime_level, set_runtime_level};
pub use record::{LogRecord, RecordMessage};
pub use sink::{FormattedRecord, LogSink};
pub use span::{SpanGuard, SpanSnapshot, enter_span};

// ── Internal emit function (called by macros) ───────────────────────────

/// Emit a pre-formatted log entry. Called by the [`klog!`] macro.
#[doc(hidden)]
#[inline]
pub fn __emit_log(
    level: Level,
    subsystem: &'static str,
    message: RecordMessage,
    file: &'static str,
    line: u32,
) {
    if hadron_core::cpu_local::cpu_is_initialized() {
        let record = LogRecord {
            timestamp: read_tsc(),
            level,
            subsystem,
            file,
            line,
            spans: span::current_spans(),
            message,
        };
        buffer::push_record(record);
    } else {
        early::emit_serial_sync(level, subsystem, &message);
    }
}

/// Formats `core::fmt::Arguments` into a fixed-size byte buffer.
///
/// Returns the number of bytes written (capped at 128, truncated if longer).
#[doc(hidden)]
pub fn __format_into_buf(buf: &mut [u8; 128], args: core::fmt::Arguments<'_>) -> u8 {
    use core::fmt::Write;

    struct BufWriter<'a> {
        buf: &'a mut [u8; 128],
        pos: usize,
    }

    impl Write for BufWriter<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = 128 - self.pos;
            let n = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
            self.pos += n;
            Ok(())
        }
    }

    let mut writer = BufWriter { buf, pos: 0 };
    let _ = core::fmt::write(&mut writer, args);
    #[allow(clippy::cast_possible_truncation)]
    let len = writer.pos.min(255) as u8;
    len
}

/// Synchronously drains all per-CPU ring buffers and dispatches to sinks.
///
/// Call this after boot initialization to flush buffered messages, or
/// from a panic handler to ensure all messages are output.
pub fn flush() {
    drain::drain_all();
}

/// Reads the Time Stamp Counter (or returns 0 on host/non-x86).
#[inline]
fn read_tsc() -> u64 {
    #[cfg(all(target_os = "none", target_arch = "x86_64"))]
    {
        // SAFETY: rdtsc is always available on x86_64. It reads a
        // monotonically-increasing timestamp counter with no side effects.
        unsafe {
            let lo: u32;
            let hi: u32;
            core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
            u64::from(hi) << 32 | u64::from(lo)
        }
    }
    #[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
    {
        0
    }
}
