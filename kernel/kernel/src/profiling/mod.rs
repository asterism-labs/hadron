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
//!
//! Call [`init`] from kernel boot to auto-start configured profilers
//! and schedule automatic drain after `PROFILE_DURATION_SECS`.

#[cfg(hadron_profile_sample)]
pub mod buffer;
#[cfg(any(hadron_profile_sample, hadron_profile_ftrace))]
pub mod format;
#[cfg(hadron_profile_ftrace)]
pub mod ftrace;
#[cfg(hadron_profile_sample)]
pub mod sample;

/// Initialize and auto-start configured profilers.
///
/// Starts the sampling profiler and/or ftrace based on Kconfig gates,
/// then spawns a background task that stops profiling and drains data
/// to serial after [`crate::config::PROFILE_DURATION_SECS`].
#[cfg(any(hadron_profile_sample, hadron_profile_ftrace))]
pub fn init() {
    #[cfg(hadron_profile_sample)]
    {
        let rate = crate::config::PROFILE_SAMPLE_RATE;
        sample::start(rate);
    }

    #[cfg(hadron_profile_ftrace)]
    {
        ftrace::start();
    }

    let duration_secs = u64::from(crate::config::PROFILE_DURATION_SECS);
    crate::sched::spawn_background("profiling-drain", async move {
        crate::sched::primitives::sleep_ms(duration_secs * 1000).await;

        crate::kinfo!("Profiling duration elapsed ({duration_secs}s), stopping...");

        #[cfg(hadron_profile_sample)]
        sample::stop();

        #[cfg(hadron_profile_ftrace)]
        ftrace::stop();

        crate::kinfo!("Profiling data emitted to serial");
    });
}
