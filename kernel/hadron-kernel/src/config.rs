//! Kernel configuration bridge.
//!
//! Re-exports constants from the generated `hadron_config` crate with
//! appropriate kernel types, providing a single source of truth for
//! compile-time configuration values.

extern crate hadron_config;

use crate::log::LogLevel;

/// Maximum kernel log level (compile-time). Sinks at or below this level
/// will receive messages; higher-verbosity messages are compiled out.
pub const MAX_LOG_LEVEL: LogLevel = match hadron_config::LOG_LEVEL.as_bytes() {
    b"error" => LogLevel::Error,
    b"warn" => LogLevel::Warn,
    b"info" => LogLevel::Info,
    b"debug" => LogLevel::Debug,
    b"trace" => LogLevel::Trace,
    _ => LogLevel::Debug,
};

/// Maximum number of CPUs supported by the kernel.
pub const MAX_CPUS: usize = hadron_config::MAX_CPUS as usize;

/// Kernel heap size in bytes.
pub const KERNEL_HEAP_SIZE: u64 = hadron_config::KERNEL_HEAP_SIZE;

/// Sampling profiler: sample rate in Hz (Kconfig default: 100).
#[cfg(hadron_profile_sample)]
pub const PROFILE_SAMPLE_RATE: u32 = hadron_config::PROFILE_SAMPLE_RATE;

/// Sampling profiler: max stack depth per sample (Kconfig default: 16).
#[cfg(hadron_profile_sample)]
pub const PROFILE_SAMPLE_DEPTH: u32 = hadron_config::PROFILE_SAMPLE_DEPTH;

/// Sampling profiler: per-CPU ring buffer entries (Kconfig default: 1024).
#[cfg(hadron_profile_sample)]
pub const PROFILE_SAMPLE_BUFFER: u32 = hadron_config::PROFILE_SAMPLE_BUFFER;

/// Function tracing: per-CPU buffer size in KB (Kconfig default: 64).
#[cfg(hadron_profile_ftrace)]
pub const PROFILE_FTRACE_BUFFER_KB: u32 = hadron_config::PROFILE_FTRACE_BUFFER_KB;

/// Profiling auto-stop duration in seconds (Kconfig default: 10).
#[cfg(any(hadron_profile_sample, hadron_profile_ftrace))]
pub const PROFILE_DURATION_SECS: u32 = hadron_config::PROFILE_DURATION_SECS;

/// Build target name.
pub const TARGET: &str = hadron_config::TARGET;

/// Build profile name.
pub const PROFILE: &str = hadron_config::PROFILE;

/// Project version.
pub const VERSION: &str = hadron_config::VERSION;
