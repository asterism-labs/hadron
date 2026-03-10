//! Time subsystem and clock source abstraction.
//!
//! Provides the [`ClockSource`] trait for hardware clock drivers and
//! compile-time stubs for time-related functions referenced by the
//! interrupt dispatch, ACPI init, and legacy init modules.

/// A hardware clock source that provides monotonic time readings.
pub trait ClockSource {
    /// Returns the current time in nanoseconds since the clock was enabled.
    fn read_nanos(&self) -> u64;
}

#[cfg(hadron_hpet)]
use crate::addr::VirtAddr;
#[cfg(hadron_hpet)]
use crate::arch::x86_64::hw::hpet::Hpet;

/// Kernel time tracking (stub).
pub struct Time;

impl Time {
    /// Record a PIT timer tick (no-op stub).
    pub fn pit_tick() {}

    /// Initialize PIT timer (no-op stub).
    pub fn init_pit() {}

    /// Initialize RTC epoch (no-op stub).
    pub fn init_rtc_epoch() {}

    /// Returns the current timer tick count (no-op stub, returns 0).
    pub fn timer_ticks() -> u64 {
        0
    }

    /// Initialize the HPET as the global time source (no-op stub).
    #[cfg(hadron_hpet)]
    pub fn init_hpet(_base: VirtAddr, _period_fs: u64) {}

    /// Register the HPET as the global clock source (no-op stub).
    #[cfg(hadron_hpet)]
    pub fn register_hpet(_hpet: Hpet) {}
}
