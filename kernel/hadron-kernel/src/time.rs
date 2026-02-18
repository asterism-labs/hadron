//! Global boot-relative time interface.
//!
//! Backed by the HPET main counter. Returns 0 before the HPET is initialized,
//! so callers always get a valid (if imprecise) timestamp.
//!
//! Also stores the HPET driver instance as a [`ClockSource`] trait object
//! for consumers that want the trait-based interface (e.g. future vDSO).

use core::sync::atomic::{AtomicU64, Ordering};

use hadron_core::sync::SpinLock;
use hadron_drivers::hpet::Hpet;

/// Global HPET driver instance, stored after ACPI init and timer calibration.
/// Provides the [`hadron_driver_api::ClockSource`] trait interface.
static HPET_DRIVER: SpinLock<Option<Hpet>> = SpinLock::new(None);

/// HPET MMIO virtual base address. Zero means "not yet initialized".
static HPET_BASE: AtomicU64 = AtomicU64::new(0);
/// HPET counter period in femtoseconds per tick.
static HPET_PERIOD_FS: AtomicU64 = AtomicU64::new(0);
/// HPET counter value at the time of initialization (boot reference).
static HPET_START: AtomicU64 = AtomicU64::new(0);

/// Initialize the time source from the HPET.
///
/// Must be called after the HPET is mapped and enabled. Stores the current
/// counter value as the boot reference point.
pub fn init_hpet(base: hadron_core::addr::VirtAddr, period_fs: u64) {
    let counter = read_hpet_counter(base.as_u64());
    HPET_START.store(counter, Ordering::Relaxed);
    HPET_PERIOD_FS.store(period_fs, Ordering::Relaxed);
    // Release fence — gates all subsequent reads via `boot_nanos`.
    HPET_BASE.store(base.as_u64(), Ordering::Release);
}

/// Returns nanoseconds elapsed since boot. Returns 0 before HPET init.
pub fn boot_nanos() -> u64 {
    let base = HPET_BASE.load(Ordering::Acquire);
    if base == 0 {
        return 0;
    }
    let current = read_hpet_counter(base);
    let start = HPET_START.load(Ordering::Relaxed);
    let period_fs = HPET_PERIOD_FS.load(Ordering::Relaxed);
    let elapsed = current.wrapping_sub(start);
    // ticks * period_fs / 1_000_000 = nanoseconds
    (elapsed as u128 * period_fs as u128 / 1_000_000) as u64
}

/// Stores the HPET driver instance for [`ClockSource`] trait access.
///
/// Called from ACPI init after timer calibration is complete.
pub fn register_hpet(hpet: Hpet) {
    *HPET_DRIVER.lock() = Some(hpet);
}

/// Runs a closure with a reference to the global [`ClockSource`].
///
/// Returns `None` if the HPET has not been registered yet.
pub fn with_clock_source<R>(f: impl FnOnce(&dyn hadron_driver_api::ClockSource) -> R) -> Option<R> {
    let guard = HPET_DRIVER.lock();
    let hpet = guard.as_ref()?;
    Some(f(hpet))
}

/// Returns the current timer tick count (1 tick ≈ 1 ms).
///
/// On x86_64, delegates to the ACPI/LAPIC timer tick counter.
/// On other architectures, derives ticks from `boot_nanos()`.
pub fn timer_ticks() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        crate::arch::x86_64::acpi::timer_ticks()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        boot_nanos() / 1_000_000
    }
}

/// Reads the HPET main counter register at offset 0xF0.
fn read_hpet_counter(base: u64) -> u64 {
    // SAFETY: `base` is the HPET MMIO virtual address set during init.
    // Offset 0xF0 is the main counter register.
    unsafe { core::ptr::read_volatile((base + 0xF0) as *const u64) }
}
