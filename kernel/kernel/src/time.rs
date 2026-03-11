//! Time subsystem: TSC calibration and timestamp conversion.
//!
//! Calibrates the TSC frequency using the PIT (channel 2 one-shot)
//! during early boot, then registers a converter with `hadron-log`
//! so that log timestamps appear as human-readable wall-clock offsets
//! from boot.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// TSC frequency in kHz (0 = uncalibrated).
static TSC_FREQ_KHZ: AtomicU32 = AtomicU32::new(0);

/// TSC value captured at the very start of `kernel_init`.
static BOOT_TSC: AtomicU64 = AtomicU64::new(0);

/// Records the boot TSC value.
///
/// Must be called at the very start of `kernel_init()`, before any
/// other code that reads TSC, to establish the time-zero reference.
pub fn record_boot_tsc() {
    let tsc = crate::arch::x86_64::hw::tsc::read_tsc();
    BOOT_TSC.store(tsc, Ordering::Relaxed);
}

/// Calibrates the TSC frequency using PIT channel 2.
///
/// Measures TSC ticks over a 10 ms PIT busy-wait, computes frequency,
/// and registers the converter with `hadron-log` for human-readable
/// timestamps.
///
/// # Safety
///
/// Must be called with interrupts disabled. The PIT must not be in use
/// by other code.
pub unsafe fn calibrate_tsc() {
    let tsc_before = crate::arch::x86_64::hw::tsc::read_tsc();

    // SAFETY: Caller guarantees interrupts are disabled and PIT is available.
    unsafe { crate::arch::x86_64::hw::pit::busy_wait_ms(10) };

    let tsc_after = crate::arch::x86_64::hw::tsc::read_tsc();
    let delta = tsc_after.wrapping_sub(tsc_before);

    // freq_khz = (delta_ticks / 10ms) * 1000 = delta_ticks * 100
    // But more precisely: delta ticks in 10ms → ticks/sec = delta * 100
    // freq_khz = delta * 100 / 1000 = delta / 10
    #[allow(clippy::cast_possible_truncation)]
    let freq_khz = (delta / 10) as u32;
    TSC_FREQ_KHZ.store(freq_khz, Ordering::Relaxed);

    hadron_log::set_tsc_converter(tsc_to_nanos_impl);

    crate::kinfo!("time", "TSC calibrated: {} MHz", freq_khz / 1000);
}

// ── Legacy stubs ─────────────────────────────────────────────────────────

/// Kernel time tracking (stub interface for legacy/ACPI paths).
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
    pub fn init_hpet(_base: hadron_core::addr::VirtAddr, _period_fs: u64) {}

    /// Register the HPET as the global clock source (no-op stub).
    #[cfg(hadron_hpet)]
    pub fn register_hpet(_hpet: crate::arch::x86_64::hw::hpet::Hpet) {}
}

/// Returns nanoseconds elapsed since boot.
///
/// Uses the calibrated TSC frequency. Returns 0 if uncalibrated.
pub fn nanos_since_boot() -> u64 {
    let tsc = crate::arch::x86_64::hw::tsc::read_tsc();
    tsc_to_nanos_impl(tsc)
}

/// Sleeps until the given monotonic deadline (nanoseconds since boot).
///
/// Returns immediately if the deadline has already passed.
#[cfg(ktest)]
pub(crate) async fn sleep_until(deadline_ns: u64) {
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll};

    struct SleepUntil {
        deadline_ns: u64,
    }

    impl Future for SleepUntil {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if nanos_since_boot() >= self.deadline_ns {
                Poll::Ready(())
            } else {
                hadron_sched::timer::register_sleep_waker(self.deadline_ns, cx.waker().clone());
                Poll::Pending
            }
        }
    }

    SleepUntil { deadline_ns }.await;
}

// ── TSC converter ────────────────────────────────────────────────────────

/// Converts a raw TSC value to nanoseconds relative to boot.
///
/// Registered with `hadron-log` as the TSC converter callback.
fn tsc_to_nanos_impl(tsc: u64) -> u64 {
    let boot = BOOT_TSC.load(Ordering::Relaxed);
    let delta = tsc.wrapping_sub(boot);
    let freq_khz = u64::from(TSC_FREQ_KHZ.load(Ordering::Relaxed));
    if freq_khz == 0 {
        return 0;
    }
    // nanos = delta * 1_000_000 / freq_khz
    // Use u128 to avoid overflow for large TSC values.
    ((u128::from(delta) * 1_000_000) / u128::from(freq_khz)) as u64
}

// ── Kernel integration tests ────────────────────────────────────────

#[cfg(ktest)]
mod ktest {
    use hadron_ktest::kernel_test;

    /// Verifies that `sleep_until` suspends for approximately 50 ms.
    #[kernel_test(stage = "with_executor")]
    async fn test_sleep_50ms() {
        let before = crate::time::nanos_since_boot();
        crate::time::sleep_until(before + 50_000_000).await;
        let elapsed = crate::time::nanos_since_boot() - before;
        assert!(
            elapsed >= 40_000_000 && elapsed < 200_000_000,
            "sleep_50ms: elapsed {elapsed}ns out of range"
        );
    }

    /// Verifies that `sleep_until` with a past deadline returns immediately.
    #[kernel_test(stage = "with_executor")]
    async fn test_sleep_past_deadline() {
        crate::time::sleep_until(0).await;
        // Should return immediately — no assertion needed beyond not hanging.
    }
}
