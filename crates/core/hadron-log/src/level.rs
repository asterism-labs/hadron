//! Severity levels for log messages.
//!
//! Levels are continuous `u8` values where lower numbers indicate higher
//! severity. Named constants are provided at conventional thresholds.

use core::fmt;
use core::sync::atomic::{AtomicU8, Ordering};

/// A log severity level.
///
/// Lower numeric values represent higher severity. The range is `0..=255`,
/// with [`Level::FATAL`] at 0 and [`Level::ALL`] at 255.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Level(pub u8);

impl Level {
    /// Unrecoverable error — the kernel cannot continue.
    pub const FATAL: Self = Self(0);
    /// Serious error that may degrade functionality.
    pub const ERROR: Self = Self(10);
    /// Unexpected condition that may indicate a problem.
    pub const WARN: Self = Self(30);
    /// General operational information.
    pub const INFO: Self = Self(50);
    /// Detailed information useful during development.
    pub const DEBUG: Self = Self(100);
    /// Very fine-grained tracing of execution flow.
    pub const TRACE: Self = Self(200);
    /// Pass-through level that enables all messages.
    pub const ALL: Self = Self(255);
}

impl fmt::Debug for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::FATAL => f.write_str("FATAL"),
            Self::ERROR => f.write_str("ERROR"),
            Self::WARN => f.write_str("WARN"),
            Self::INFO => f.write_str("INFO"),
            Self::DEBUG => f.write_str("DEBUG"),
            Self::TRACE => f.write_str("TRACE"),
            other => write!(f, "Level({:#x})", other.0),
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

// ── Compile-time maximum level ──────────────────────────────────────────

/// Returns the compile-time maximum log level.
///
/// Reads from `--cfg hadron_log_max_level="N"`. Defaults to 255 (all).
#[doc(hidden)]
#[macro_export]
macro_rules! __hadron_log_max_level {
    () => {{
        #[allow(unreachable_code)]
        {
            #[cfg(hadron_log_max_level = "0")]
            {
                0u8
            }
            #[cfg(hadron_log_max_level = "10")]
            {
                10u8
            }
            #[cfg(hadron_log_max_level = "30")]
            {
                30u8
            }
            #[cfg(hadron_log_max_level = "50")]
            {
                50u8
            }
            #[cfg(hadron_log_max_level = "100")]
            {
                100u8
            }
            #[cfg(hadron_log_max_level = "200")]
            {
                200u8
            }
            #[cfg(hadron_log_max_level = "255")]
            {
                255u8
            }
            #[cfg(not(any(
                hadron_log_max_level = "0",
                hadron_log_max_level = "10",
                hadron_log_max_level = "30",
                hadron_log_max_level = "50",
                hadron_log_max_level = "100",
                hadron_log_max_level = "200",
                hadron_log_max_level = "255",
            )))]
            {
                255u8
            }
        }
    }};
}

// ── Runtime level filter ────────────────────────────────────────────────

/// Global runtime log level. Messages with severity > this value are dropped.
static RUNTIME_LEVEL: AtomicU8 = AtomicU8::new(255);

/// Returns a reference to the runtime log level atomic.
///
/// Used by the log macros for the fast-path runtime check.
#[doc(hidden)]
#[inline]
pub fn runtime_level() -> &'static AtomicU8 {
    &RUNTIME_LEVEL
}

/// Sets the runtime log level filter.
///
/// Messages with a numeric severity greater than `level` will be dropped.
/// This takes effect immediately on all CPUs.
pub fn set_runtime_level(level: Level) {
    RUNTIME_LEVEL.store(level.0, Ordering::Relaxed);
}

/// Returns the current runtime log level.
pub fn get_runtime_level() -> Level {
    Level(RUNTIME_LEVEL.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ordering() {
        assert!(Level::FATAL < Level::ERROR);
        assert!(Level::ERROR < Level::WARN);
        assert!(Level::WARN < Level::INFO);
        assert!(Level::INFO < Level::DEBUG);
        assert!(Level::DEBUG < Level::TRACE);
        assert!(Level::TRACE < Level::ALL);
    }

    #[test]
    fn level_display() {
        assert_eq!(format!("{}", Level::FATAL), "FATAL");
        assert_eq!(format!("{}", Level::INFO), "INFO");
        assert_eq!(format!("{}", Level(42)), "Level(0x2a)");
    }

    #[test]
    fn runtime_level_set_get() {
        set_runtime_level(Level::WARN);
        assert_eq!(get_runtime_level(), Level::WARN);
        // Reset for other tests
        set_runtime_level(Level::ALL);
    }
}
