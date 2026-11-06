//! The HPET, the High Precision Event Timer, as a clock source.
//!
//! The ACPI HPET table gives the address of a 1 KiB register block. The
//! General Capabilities register gives the main counter's period and
//! width. [`init`] turns its comparators' interrupts off, turns legacy
//! replacement routing off, and starts the main counter. Register offsets
//! and bits are from the IA-PC HPET Specification 1.0a.

use core::fmt;

use arch_x86_64::VirtAddr;

use crate::acpi;
use crate::memory;
use crate::sync::WriteOnce;
use crate::time::ClockSource;

/// The size of the register block.
const REGISTER_BLOCK: u64 = 1024;

/// General Capabilities and ID, read-only.
const CAPABILITIES: usize = 0x000;
/// General Configuration.
const CONFIG: usize = 0x010;
/// The main counter.
const MAIN_COUNTER: usize = 0x0f0;

/// Timer `n`'s Configuration and Capabilities register.
const fn timer_config(n: usize) -> usize {
    0x100 + 0x20 * n
}

/// Capabilities bits 8-12: the number of comparators, minus one.
const CAP_TIMERS_SHIFT: u32 = 8;
const CAP_TIMERS_MASK: u64 = 0x1f;
/// Capabilities bit 13: the main counter is 64 bits wide.
const CAP_COUNTER_64: u64 = 1 << 13;
/// Capabilities bits 32-63: the main counter's period in femtoseconds.
const CAP_PERIOD_SHIFT: u32 = 32;
/// The longest period the specification allows, 100 ns.
const MAX_PERIOD_FS: u32 = 100_000_000;
const FEMTOS_PER_SEC: u64 = 1_000_000_000_000_000;

/// Configuration bit 0: the main counter runs and comparators may fire.
const CONFIG_ENABLE: u64 = 1 << 0;
/// Configuration bit 1: timers 0 and 1 replace the PIT and RTC IRQs.
const CONFIG_LEGACY: u64 = 1 << 1;

/// Timer configuration bit 2: the comparator raises interrupts.
const TIMER_INT_ENABLE: u64 = 1 << 2;

/// The HPET's main counter, counted from [`init`].
#[derive(Debug)]
pub(crate) struct HpetClock {
    registers: VirtAddr,
    /// The count when [`init`] ran.
    base: u64,
    /// Counts per second.
    frequency: u64,
    /// How many comparators the HPET has.
    timers: usize,
}

static HPET: WriteOnce<HpetClock> = WriteOnce::new();

/// Why the kernel does not use the HPET.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unusable {
    /// ACPI lists no HPET in memory.
    Missing,
    /// The main counter is 32 bits wide, so it wraps within minutes.
    Counter32Bit,
    /// The period is zero or longer than 100 ns.
    BadPeriod(u32),
}

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("ACPI lists no HPET"),
            Self::Counter32Bit => f.write_str("the HPET counter is 32 bits wide"),
            Self::BadPeriod(fs) => write!(f, "the HPET reports a period of {fs} fs"),
        }
    }
}

fn read(registers: VirtAddr, offset: usize) -> u64 {
    // SAFETY: `registers` is the mapped register block, and every offset
    // here is an aligned 64-bit register inside it.
    unsafe { (registers + offset as u64).as_ptr::<u64>().read_volatile() }
}

fn write(registers: VirtAddr, offset: usize, value: u64) {
    // SAFETY: as in `read`. Only `init` writes, once, at boot.
    unsafe {
        (registers + offset as u64)
            .as_mut_ptr::<u64>()
            .write_volatile(value);
    }
}

/// Maps the HPET that ACPI lists, checks it, and starts its main counter
/// with every comparator's interrupt off. Boot calls this once.
///
/// # Errors
///
/// Returns why the HPET is not fit to keep time.
///
/// # Panics
///
/// Panics before `acpi::init`, or if called twice.
pub(crate) fn init() -> Result<&'static HpetClock, Unusable> {
    let table = acpi::hpet().ok_or(Unusable::Missing)?;
    let registers = memory::map_mmio(table.address, REGISTER_BLOCK);
    let capabilities = read(registers, CAPABILITIES);
    #[allow(clippy::cast_possible_truncation)] // the top 32 bits
    let period = (capabilities >> CAP_PERIOD_SHIFT) as u32;
    if period == 0 || period > MAX_PERIOD_FS {
        return Err(Unusable::BadPeriod(period));
    }
    if capabilities & CAP_COUNTER_64 == 0 {
        return Err(Unusable::Counter32Bit);
    }
    #[allow(clippy::cast_possible_truncation)] // 5 bits
    let timers = ((capabilities >> CAP_TIMERS_SHIFT) & CAP_TIMERS_MASK) as usize + 1;

    // Stop the counter while the comparators change, so none fires on a
    // half-made setup.
    let config = read(registers, CONFIG) & !CONFIG_LEGACY;
    write(registers, CONFIG, config & !CONFIG_ENABLE);
    for n in 0..timers {
        let timer = read(registers, timer_config(n));
        write(registers, timer_config(n), timer & !TIMER_INT_ENABLE);
    }
    write(registers, CONFIG, config | CONFIG_ENABLE);

    Ok(HPET.init(HpetClock {
        registers,
        base: read(registers, MAIN_COUNTER),
        frequency: FEMTOS_PER_SEC / u64::from(period),
        timers,
    }))
}

impl HpetClock {
    /// How many comparators the HPET has.
    pub(crate) fn timers(&self) -> usize {
        self.timers
    }
}

impl ClockSource for HpetClock {
    fn name(&self) -> &'static str {
        "HPET"
    }

    fn read(&self) -> u64 {
        read(self.registers, MAIN_COUNTER) - self.base
    }

    fn frequency(&self) -> u64 {
        self.frequency
    }
}

ktest::ktest! {
    fn hpet_measures_a_pit_wait() {
        use super::pit;

        const WAITS: u64 = 4;
        let hpet = HPET.get().expect("boot started q35's HPET");
        // QEMU's HPET runs at 100 MHz with 3 comparators.
        assert_eq!(hpet.frequency(), 100_000_000);
        assert_eq!(hpet.timers(), 3);
        let start = hpet.read();
        for _ in 0..WAITS {
            pit::wait_ms(pit::MAX_WAIT_MS, || {});
        }
        let ms = (hpet.read() - start) * 1000 / hpet.frequency();
        let expected = WAITS * u64::from(pit::MAX_WAIT_MS);
        // QEMU's timers drift against each other under host load, so the
        // check allows a wide margin.
        assert!(
            (expected / 2..=expected * 2).contains(&ms),
            "measured {ms} ms for a {expected} ms wait"
        );
    }
}
