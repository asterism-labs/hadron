//! Hardware abstraction traits for interrupt controllers, clocks, and timers.
//!
//! These traits define portable interfaces for hardware subsystems.
//! Implementations live in `hadron-drivers`; kernel code accesses hardware
//! through these trait interfaces where feasible.

/// An interrupt controller that can send EOI and mask/unmask IRQ lines.
pub trait InterruptController: Send + Sync {
    /// Sends an End-of-Interrupt signal for the given vector.
    fn send_eoi(&self);

    /// Masks (disables) the IRQ line at the given index.
    fn mask_irq(&self, irq: u8);

    /// Unmasks (enables) the IRQ line at the given index.
    fn unmask_irq(&self, irq: u8);
}

/// A monotonic clock source that provides a nanosecond timestamp.
pub trait ClockSource: Send + Sync {
    /// Returns the current time in nanoseconds since an arbitrary epoch.
    fn read_nanos(&self) -> u64;
}

/// A hardware timer that can generate periodic interrupts.
pub trait Timer: Send + Sync {
    /// Starts the timer in periodic mode with the given interval.
    fn set_periodic(&mut self, interval_ns: u64);

    /// Stops the timer.
    fn stop(&mut self);
}

/// A hardware watchdog timer that resets the system on expiry.
pub trait Watchdog: Send + Sync {
    /// Arms the watchdog with a timeout in seconds. Countdown begins immediately.
    fn arm(&self, timeout_secs: u32);
    /// Pets (reloads) the watchdog, resetting the countdown.
    fn pet(&self);
    /// Disarms the watchdog, stopping the countdown.
    fn disarm(&self);
}
