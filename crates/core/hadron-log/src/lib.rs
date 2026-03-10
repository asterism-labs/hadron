//! Structured logging subsystem for the Hadron kernel.
//!
//! Provides severity-based logging with subsystem tagging, a global
//! lock-free MPSC ring buffer, span-scoped context, and linkset-based
//! sink registration. All messages are formatted at the call site using
//! `core::fmt` into a fixed 128-byte inline buffer.
//!
//! # Architecture
//!
//! Every `klog!` invocation pushes a [`LogRecord`] into a single global
//! [`MpscRing`](ring::MpscRing). By default, auto-flush is enabled and
//! each push immediately drains the ring through all registered sinks.
//! Call [`disable_auto_flush()`] once a periodic drain mechanism (e.g.
//! a timer or scheduler tick) is in place.
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

mod drain;
mod fmt;
mod level;
mod macros;
mod record;
pub(crate) mod ring;
mod sink;
mod span;

pub use level::{Level, get_runtime_level, runtime_level, set_runtime_level};
pub use record::{LogRecord, RecordMessage};
pub use sink::{FormattedRecord, LogSink};
pub use span::{SpanGuard, SpanSnapshot, enter_span};

use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

// ── TSC-to-nanoseconds converter ─────────────────────────────────────────

/// Function pointer that converts a raw TSC delta to nanoseconds.
///
/// Registered by the kernel once TSC frequency is calibrated. Before
/// registration, timestamps are rendered as raw hex TSC values.
static TSC_CONVERTER: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Registers a TSC-to-nanoseconds converter function.
///
/// The converter receives a raw TSC value (already offset from boot TSC)
/// and returns nanoseconds. Call this once after calibrating TSC frequency.
pub fn set_tsc_converter(f: fn(u64) -> u64) {
    TSC_CONVERTER.store(f as *mut (), Ordering::Release);
}

/// Converts a raw TSC value to nanoseconds using the registered converter.
///
/// Returns `None` if no converter has been registered yet (pre-calibration).
pub(crate) fn tsc_to_nanos(tsc: u64) -> Option<u64> {
    let ptr = TSC_CONVERTER.load(Ordering::Acquire);
    if ptr.is_null() {
        None
    } else {
        // SAFETY: The pointer was stored via `set_tsc_converter` which takes
        // a `fn(u64) -> u64`. We cast it back to the same function type.
        let f: fn(u64) -> u64 = unsafe { core::mem::transmute(ptr) };
        Some(f(tsc))
    }
}

// ── Auto-flush control ──────────────────────────────────────────────────

/// When `true`, every `__emit_log` call immediately drains the ring.
///
/// Enabled by default so that early boot messages appear on serial
/// without an explicit `flush()`. Disable once a periodic drain
/// mechanism is running.
static AUTO_FLUSH: AtomicBool = AtomicBool::new(true);

/// Disables auto-flush after each log call.
///
/// Call this once a periodic drain mechanism (timer tick, scheduler
/// idle loop) is in place. After this, log records accumulate in the
/// ring until [`flush()`] is called explicitly.
pub fn disable_auto_flush() {
    AUTO_FLUSH.store(false, Ordering::Relaxed);
}

/// Re-enables auto-flush after each log call.
///
/// Used when a final message must be guaranteed to reach serial output
/// (e.g. before halting the CPU on task exit).
pub fn enable_auto_flush() {
    AUTO_FLUSH.store(true, Ordering::Relaxed);
}

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
    let record = LogRecord {
        timestamp: read_tsc(),
        level,
        subsystem,
        file,
        line,
        spans: span::current_spans(),
        message,
    };
    ring::RING.push(record);

    if AUTO_FLUSH.load(Ordering::Relaxed) {
        drain::drain_all();
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

/// Synchronously drains the global ring buffer and dispatches to sinks.
///
/// Call this from a panic handler or anywhere immediate output is needed.
/// With auto-flush enabled (the default), this is also called after every
/// log emission.
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
