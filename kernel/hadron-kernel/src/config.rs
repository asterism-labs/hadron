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

/// Build target name.
pub const TARGET: &str = hadron_config::TARGET;

/// Build profile name.
pub const PROFILE: &str = hadron_config::PROFILE;

/// Project version.
pub const VERSION: &str = hadron_config::VERSION;
