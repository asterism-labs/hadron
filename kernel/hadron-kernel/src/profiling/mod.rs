//! Kernel profiling infrastructure.
//!
//! Provides sampling profiler and function tracing capabilities behind
//! Kconfig-controlled feature gates:
//!
//! - `profile_sample` — periodic timer-interrupt-driven RIP+stack capture
//! - `profile_ftrace` — `mcount`-based function entry instrumentation
//!
//! Both emit data to serial in a shared binary format (HPRF) parsed by
//! gluon's `perf` analysis modules.

#[cfg(hadron_profile_sample)]
pub mod buffer;
#[cfg(any(hadron_profile_sample, hadron_profile_ftrace))]
pub mod format;
#[cfg(hadron_profile_sample)]
pub mod sample;
#[cfg(hadron_profile_ftrace)]
pub mod ftrace;
