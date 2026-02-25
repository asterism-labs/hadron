//! Global boot-relative time interface.
//!
//! Backed by the HPET main counter when available, or a PIT tick counter as
//! fallback. Returns 0 before the time source is initialized, so callers
//! always get a valid (if imprecise) timestamp.
//!
//! Also stores the HPET driver instance as a [`ClockSource`] trait object
//! for consumers that want the trait-based interface (e.g. future vDSO).

use hadron_core::sync::atomic::{AtomicU64, Ordering};

#[cfg(hadron_hpet)]
use crate::arch::x86_64::hw::hpet::Hpet;
#[cfg(hadron_hpet)]
use crate::sync::SpinLock;

/// Global HPET driver instance, stored after ACPI init and timer calibration.
/// Provides the [`crate::driver_api::ClockSource`] trait interface.
#[cfg(hadron_hpet)]
static HPET_DRIVER: SpinLock<Option<Hpet>> = SpinLock::leveled("HPET_DRIVER", 4, None);

/// HPET MMIO virtual base address. Zero means "not yet initialized".
#[cfg(hadron_hpet)]
static HPET_BASE: AtomicU64 = AtomicU64::new(0);
/// HPET counter period in femtoseconds per tick.
#[cfg(hadron_hpet)]
static HPET_PERIOD_FS: AtomicU64 = AtomicU64::new(0);
/// HPET counter value at the time of initialization (boot reference).
#[cfg(hadron_hpet)]
static HPET_START: AtomicU64 = AtomicU64::new(0);

/// PIT tick counter, incremented on each IRQ 0 (PIT mode only).
static PIT_TICKS: AtomicU64 = AtomicU64::new(0);
/// Nanoseconds per PIT tick (set by [`Time::init_pit`]).
static PIT_NANOS_PER_TICK: AtomicU64 = AtomicU64::new(0);

/// Unix epoch seconds at the time of boot (from CMOS RTC).
static BOOT_EPOCH_SECS: AtomicU64 = AtomicU64::new(0);

/// Zero-sized facade for the global time subsystem.
pub struct Time;

impl Time {
    /// Initialize the time source from the HPET.
    ///
    /// Must be called after the HPET is mapped and enabled. Stores the current
    /// counter value as the boot reference point.
    #[cfg(hadron_hpet)]
    pub fn init_hpet(base: crate::addr::VirtAddr, period_fs: u64) {
        let counter = read_hpet_counter(base.as_u64());
        HPET_START.store(counter, Ordering::Relaxed);
        HPET_PERIOD_FS.store(period_fs, Ordering::Relaxed);
        // Release fence — gates all subsequent reads via `boot_nanos`.
        HPET_BASE.store(base.as_u64(), Ordering::Release);
    }

    /// Initialize the PIT-based tick counter.
    ///
    /// At the configured PIT frequency (typically 1000 Hz), each tick is 1 ms.
    /// Time is tracked by incrementing [`PIT_TICKS`] on each IRQ 0.
    pub fn init_pit() {
        // At 1000 Hz, each tick is 1 ms = 1_000_000 ns.
        PIT_NANOS_PER_TICK.store(1_000_000, Ordering::Release);
    }

    /// Increment the PIT tick counter. Called from the timer IRQ handler.
    pub fn pit_tick() {
        PIT_TICKS.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns nanoseconds elapsed since boot.
    ///
    /// Uses HPET if available, otherwise falls back to PIT tick count.
    /// Returns 0 before any time source is initialized.
    pub fn boot_nanos() -> u64 {
        // Try HPET first (high precision).
        #[cfg(hadron_hpet)]
        {
            let base = HPET_BASE.load(Ordering::Acquire);
            if base != 0 {
                let current = read_hpet_counter(base);
                let start = HPET_START.load(Ordering::Relaxed);
                let period_fs = HPET_PERIOD_FS.load(Ordering::Relaxed);
                let elapsed = current.wrapping_sub(start);
                // ticks * period_fs / 1_000_000 = nanoseconds
                return (elapsed as u128 * period_fs as u128 / 1_000_000) as u64;
            }
        }

        // Fall back to PIT tick counter.
        let nanos_per_tick = PIT_NANOS_PER_TICK.load(Ordering::Acquire);
        if nanos_per_tick != 0 {
            return PIT_TICKS.load(Ordering::Relaxed) * nanos_per_tick;
        }

        0
    }

    /// Initialize the wall-clock epoch from the CMOS RTC.
    ///
    /// Must be called early in boot (interrupts disabled). Stores the
    /// Unix epoch seconds at boot time for `CLOCK_REALTIME`.
    pub fn init_rtc_epoch() {
        // SAFETY: Called early in boot with interrupts disabled.
        let epoch = unsafe { crate::arch::x86_64::hw::rtc::read_rtc() };
        BOOT_EPOCH_SECS.store(epoch, Ordering::Release);
    }

    /// Returns the current wall-clock time as nanoseconds since the Unix epoch.
    ///
    /// Computed as: `(boot_epoch_seconds * 1e9) + boot_nanos()`.
    /// Returns 0 if the RTC has not been initialized.
    pub fn realtime_nanos() -> u64 {
        let epoch_secs = BOOT_EPOCH_SECS.load(Ordering::Acquire);
        if epoch_secs == 0 {
            return 0;
        }
        epoch_secs
            .saturating_mul(1_000_000_000)
            .saturating_add(Self::boot_nanos())
    }

    /// Stores the HPET driver instance for [`ClockSource`] trait access.
    ///
    /// Called from ACPI init after timer calibration is complete.
    #[cfg(hadron_hpet)]
    pub fn register_hpet(hpet: Hpet) {
        *HPET_DRIVER.lock() = Some(hpet);
    }

    /// Runs a closure with a reference to the global [`ClockSource`].
    ///
    /// Returns `None` if the HPET has not been registered yet.
    #[cfg(hadron_hpet)]
    pub fn with_clock_source<R>(
        f: impl FnOnce(&dyn crate::driver_api::ClockSource) -> R,
    ) -> Option<R> {
        let guard = HPET_DRIVER.lock();
        let hpet = guard.as_ref()?;
        Some(f(hpet))
    }

    /// Returns the current timer tick count (1 tick ≈ 1 ms).
    ///
    /// Derived from [`boot_nanos`](Self::boot_nanos), so the result is
    /// SMP-safe and consistent with log timestamps regardless of CPU count.
    pub fn timer_ticks() -> u64 {
        Self::boot_nanos() / 1_000_000
    }
}

/// Reads the HPET main counter register at offset 0xF0.
#[cfg(hadron_hpet)]
fn read_hpet_counter(base: u64) -> u64 {
    // SAFETY: `base` is the HPET MMIO virtual address set during init.
    // Offset 0xF0 is the main counter register.
    unsafe { core::ptr::read_volatile((base + 0xF0) as *const u64) }
}
