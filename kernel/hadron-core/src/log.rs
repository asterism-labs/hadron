//! Logging interface for the Hadron kernel ecosystem.
//!
//! Provides [`kprint!`] / [`kprintln!`] macros for raw output and [`klog!`] /
//! convenience macros (`kinfo!`, `kdebug!`, etc.) for leveled, timestamped
//! logging. Before [`set_print_fn`] / [`set_log_fn`] are called, output is
//! silently discarded.

use core::fmt;
use core::sync::atomic::{AtomicPtr, Ordering};

// ---------------------------------------------------------------------------
// Log levels — lower = more severe
// ---------------------------------------------------------------------------

/// Kernel log severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    /// Fatal: unrecoverable error, system will halt.
    Fatal = 0,
    /// Error: something failed but the system may continue.
    Error = 1,
    /// Warning: unexpected condition, not necessarily an error.
    Warn = 2,
    /// Informational: high-level progress messages.
    Info = 3,
    /// Debug: detailed diagnostic information.
    Debug = 4,
    /// Trace: very verbose, low-level tracing.
    Trace = 5,
}

impl LogLevel {
    /// Returns the human-readable name (fixed-width for aligned output).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fatal => "FATAL",
            Self::Error => "ERROR",
            Self::Warn => "WARN ",
            Self::Info => "INFO ",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

// ---------------------------------------------------------------------------
// Raw print function (kprint! / kprintln!) — no levels, no filtering
// ---------------------------------------------------------------------------

/// The signature of the global print function.
pub type PrintFn = fn(fmt::Arguments<'_>);

fn null_print(_args: fmt::Arguments<'_>) {}

static PRINT_FN: AtomicPtr<()> = AtomicPtr::new(null_print as *mut ());

/// Registers the global print function.
///
/// # Safety
///
/// The provided function must be safe to call from any context. May be called
/// more than once (e.g., once for early serial, once for the full logger).
/// Uses `Release` ordering so subsequent loads see the new function.
pub unsafe fn set_print_fn(f: PrintFn) {
    PRINT_FN.store(f as *mut (), Ordering::Release);
}

/// Loads the current print function from the atomic pointer.
///
/// # Safety
///
/// Relies on the invariant that only valid `PrintFn` pointers (or the
/// initial `null_print`) are ever stored into `PRINT_FN`.
#[inline]
fn load_print_fn() -> PrintFn {
    let ptr = PRINT_FN.load(Ordering::Acquire);
    // SAFETY: We only ever store valid `PrintFn` function pointers into PRINT_FN.
    unsafe { core::mem::transmute(ptr) }
}

/// Implementation detail for [`kprint!`] / [`kprintln!`]. Not public API.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    load_print_fn()(args);
}

/// Prints to the kernel log sinks (raw, no level, no timestamp).
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::log::_print(format_args!($($arg)*)) };
}

/// Prints to the kernel log sinks with a trailing newline (raw, no level).
#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($($arg:tt)*) => { $crate::kprint!("{}\n", format_args!($($arg)*)) };
}

// ---------------------------------------------------------------------------
// Leveled log function (klog! and convenience macros)
// ---------------------------------------------------------------------------

/// The signature of the global leveled log function.
pub type LogFn = fn(LogLevel, fmt::Arguments<'_>);

fn null_log(_level: LogLevel, _args: fmt::Arguments<'_>) {}

static LOG_FN: AtomicPtr<()> = AtomicPtr::new(null_log as *mut ());

/// Registers the global leveled log function.
///
/// # Safety
///
/// The provided function must be safe to call from any context. May be called
/// more than once (e.g., once for early serial, once for the full logger).
/// Uses `Release` ordering so subsequent loads see the new function.
pub unsafe fn set_log_fn(f: LogFn) {
    LOG_FN.store(f as *mut (), Ordering::Release);
}

/// Loads the current log function from the atomic pointer.
///
/// # Safety
///
/// Same invariant as [`load_print_fn`] — only valid `LogFn` pointers are stored.
#[inline]
fn load_log_fn() -> LogFn {
    let ptr = LOG_FN.load(Ordering::Acquire);
    // SAFETY: We only ever store valid `LogFn` function pointers into LOG_FN.
    unsafe { core::mem::transmute(ptr) }
}

/// Implementation detail for [`klog!`]. Not public API.
#[doc(hidden)]
pub fn _log(level: LogLevel, args: fmt::Arguments<'_>) {
    load_log_fn()(level, args);
}

/// Logs a message at the given level.
#[macro_export]
macro_rules! klog {
    ($level:expr, $($arg:tt)*) => {
        $crate::log::_log($level, format_args!($($arg)*))
    };
}

/// Logs a fatal-level message (level 0).
#[macro_export]
macro_rules! kfatal {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Fatal, $($arg)*) };
}

/// Logs an error-level message (level 1).
#[macro_export]
macro_rules! kerr {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Error, $($arg)*) };
}

/// Logs a warning-level message (level 2).
#[macro_export]
macro_rules! kwarn {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Warn, $($arg)*) };
}

/// Logs an info-level message (level 3).
#[macro_export]
macro_rules! kinfo {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Info, $($arg)*) };
}

/// Logs a debug-level message (level 4).
#[macro_export]
macro_rules! kdebug {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Debug, $($arg)*) };
}

/// Logs a trace-level message (level 5).
#[macro_export]
macro_rules! ktrace {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Trace, $($arg)*) };
}
