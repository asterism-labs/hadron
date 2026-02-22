//! Verbose/debug logging for build system diagnostics.
//!
//! Three output levels controlled by CLI flags:
//! - **Quiet** (`-q`): errors + final summary only
//! - **Default** (no flag): "Compiling" lines + errors + summary
//! - **Verbose** (`-v`): everything — stale reasons, timings, skip lines, cache diagnostics

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

/// Output verbosity level.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verbosity {
    Quiet = 0,
    Default = 1,
    Verbose = 2,
}

/// Global verbosity level, set once at startup.
static VERBOSITY: AtomicU8 = AtomicU8::new(1); // Default

/// Initialize the verbosity level for the current process.
pub fn init(quiet: bool, verbose: bool) {
    let level = if quiet {
        Verbosity::Quiet
    } else if verbose {
        Verbosity::Verbose
    } else {
        Verbosity::Default
    };
    VERBOSITY.store(level as u8, Ordering::Relaxed);
}

/// Returns the current verbosity level.
pub fn verbosity() -> Verbosity {
    match VERBOSITY.load(Ordering::Relaxed) {
        0 => Verbosity::Quiet,
        2 => Verbosity::Verbose,
        _ => Verbosity::Default,
    }
}

/// Returns `true` if verbose mode is active.
pub fn is_verbose() -> bool {
    verbosity() == Verbosity::Verbose
}

/// Returns `true` if quiet mode is active.
pub fn is_quiet() -> bool {
    verbosity() == Verbosity::Quiet
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

/// Print a message at default verbosity and above (suppressed in quiet mode).
///
/// Usage mirrors `println!`:
/// ```ignore
/// dprintln!("  Compiling {}...", name);
/// ```
macro_rules! dprintln {
    ($($arg:tt)*) => {
        if !$crate::verbose::is_quiet() {
            println!($($arg)*);
        }
    };
}

pub(crate) use dprintln;

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
