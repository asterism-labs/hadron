//! The time stamp counter as a clock source.
//!
//! The kernel uses the TSC only when it is invariant, CPUID.80000007H:EDX
//! bit 8. An invariant TSC runs at a constant rate in every ACPI P-, C-
//! and T-state (SDM Vol. 3B §20.17.1). Without that bit the rate can
//! change with the core clock, so the count is no measure of time.
//!
//! The rate is measured against the PIT at boot.

use core::arch::asm;
use core::arch::x86_64::__cpuid;
use core::fmt;

use super::pit;
use crate::sync::WriteOnce;
use crate::time::ClockSource;

/// CPUID.01H:EDX bit 4: the CPU has a TSC (Vol. 2A, CPUID).
const CPUID_TSC: u32 = 1 << 4;
/// CPUID.80000007H:EDX bit 8: the TSC is invariant (Vol. 2A, CPUID).
const CPUID_INVARIANT_TSC: u32 = 1 << 8;

/// The TSC, counted from [`init`].
#[derive(Debug)]
pub(crate) struct TscClock {
    /// The count when [`init`] ran.
    base: u64,
    /// Counts per second.
    frequency: u64,
}

static TSC: WriteOnce<TscClock> = WriteOnce::new();

/// Why the kernel does not use the TSC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unusable {
    /// The CPU has no TSC.
    Missing,
    /// The TSC's rate may change with the core clock.
    NotInvariant,
}

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "the CPU has no TSC",
            Self::NotInvariant => "the TSC is not invariant",
        })
    }
}

/// Checks that the TSC is invariant.
///
/// # Errors
///
/// Returns why the TSC is not fit to keep time.
fn check() -> Result<(), Unusable> {
    if __cpuid(1).edx & CPUID_TSC == 0 {
        return Err(Unusable::Missing);
    }
    let max_extended = __cpuid(0x8000_0000).eax;
    if max_extended < 0x8000_0007 || __cpuid(0x8000_0007).edx & CPUID_INVARIANT_TSC == 0 {
        return Err(Unusable::NotInvariant);
    }
    Ok(())
}

/// Checks the TSC and measures its rate. Boot calls this once, with
/// interrupts masked so the measurement is not stretched.
///
/// # Errors
///
/// Returns why the TSC is not fit to keep time.
///
/// # Panics
///
/// Panics if called twice.
pub(crate) fn init() -> Result<&'static TscClock, Unusable> {
    check()?;
    let frequency = calibrate();
    Ok(TSC.init(TscClock {
        base: read(),
        frequency,
    }))
}

/// Reads the TSC. `lfence` holds `rdtsc` back until every earlier
/// instruction has finished, so reads stay in program order (Vol. 2B,
/// RDTSC).
fn read() -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: `lfence` and `rdtsc` only read the TSC into EDX:EAX.
    // CR4.TSD limits `rdtsc` outside ring 0, and the kernel is in ring 0.
    unsafe {
        asm!(
            "lfence",
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    u64::from(high) << 32 | u64::from(low)
}

/// Counts the TSC across the PIT's longest wait and returns counts per
/// second.
fn calibrate() -> u64 {
    let mut start = 0;
    pit::wait_ms(pit::MAX_WAIT_MS, || start = read());
    let elapsed = read() - start;
    elapsed * 1000 / u64::from(pit::MAX_WAIT_MS)
}

impl ClockSource for TscClock {
    fn name(&self) -> &'static str {
        "TSC"
    }

    fn read(&self) -> u64 {
        read() - self.base
    }

    fn frequency(&self) -> u64 {
        self.frequency
    }
}

ktest::ktest! {
    fn tsc_measures_a_pit_wait() {
        const WAITS: u64 = 4;
        // QEMU's emulator has no invariant TSC, so boot may not use it,
        // but the count and the calibration still work.
        assert_ne!(__cpuid(1).edx & CPUID_TSC, 0, "the CPU has a TSC");
        let frequency = calibrate();
        assert!(frequency > 0);
        let start = read();
        for _ in 0..WAITS {
            pit::wait_ms(pit::MAX_WAIT_MS, || {});
        }
        let ms = (read() - start) * 1000 / frequency;
        let expected = WAITS * u64::from(pit::MAX_WAIT_MS);
        // QEMU's timers drift against each other under host load, so the
        // check allows a wide margin.
        assert!(
            (expected / 2..=expected * 2).contains(&ms),
            "measured {ms} ms for a {expected} ms wait"
        );
    }
}
