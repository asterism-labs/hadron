//! Minimal time subsystem stubs.
//!
//! Provides compile-time stubs for `Time::pit_tick()`, `Time::init_pit()`,
//! and `Time::init_rtc_epoch()` referenced by the interrupt dispatch and
//! legacy init modules.

/// Kernel time tracking (stub).
pub struct Time;

impl Time {
    /// Record a PIT timer tick (no-op stub).
    pub fn pit_tick() {}

    /// Initialize PIT timer (no-op stub).
    pub fn init_pit() {}

    /// Initialize RTC epoch (no-op stub).
    pub fn init_rtc_epoch() {}
}
