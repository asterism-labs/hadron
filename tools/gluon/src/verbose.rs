//! Verbose/debug logging for build system diagnostics.
//!
//! When enabled via `--verbose`/`-v`, prints timing information, cache
//! decisions, and configuration details. Zero-cost when disabled.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Global verbose flag, set once at startup.
static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Enable verbose output for the current process.
pub fn init(enabled: bool) {
    VERBOSE.store(enabled, Ordering::Relaxed);
}

/// Returns `true` if verbose mode is active.
pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Print a message only when verbose mode is enabled.
///
/// Usage mirrors `println!`:
/// ```ignore
/// vprintln!("loaded {} kconfig files", count);
/// ```
macro_rules! vprintln {
    ($($arg:tt)*) => {
        if $crate::verbose::is_verbose() {
            println!($($arg)*);
        }
    };
}

pub(crate) use vprintln;

/// RAII timer that prints elapsed duration on drop when verbose mode is active.
///
/// ```ignore
/// let _t = Timer::start("script evaluation");
/// // ... work ...
/// // prints "  script evaluation: 42ms" on drop
/// ```
pub struct Timer {
    label: &'static str,
    start: Instant,
}

impl Timer {
    /// Begin timing a labeled operation.
    pub fn start(label: &'static str) -> Self {
        Self {
            label,
            start: Instant::now(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if is_verbose() {
            let elapsed = self.start.elapsed();
            println!("  {}: {:.1?}", self.label, elapsed);
        }
    }
}
