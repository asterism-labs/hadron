//! The local APIC, in xAPIC mode: its registers, end of interrupt, and a
//! periodic timer that asks the scheduler to switch threads.
//!
//! [`init`] maps the registers, enables the APIC, wires LINT0 and LINT1 as
//! the MADT says, calibrates the timer against the PIT and starts it at
//! [`HZ`]. Register offsets and bits are from SDM Vol. 3A chapter 13.

use core::sync::atomic::{AtomicU64, Ordering};

use arch_x86_64::VirtAddr;
use arch_x86_64::structures::paging::PAGE_SIZE;

use super::{cpu, irq, pit};
use crate::acpi::Madt;
use crate::memory;
use crate::percpu::{self, IrqGuard};
use crate::sync::WriteOnce;

/// The vector the APIC delivers when it drops an interrupt. Its low four
/// bits are all set, which older APICs require.
pub(crate) const SPURIOUS: u8 = 0xff;
/// The timer's vector.
pub(crate) const TIMER: u8 = 0xf0;
/// Timer interrupts per second.
pub(crate) const HZ: u64 = 100;

/// `IA32_APIC_BASE` bit 10: x2APIC mode.
const BASE_X2APIC: u64 = 1 << 10;
/// `IA32_APIC_BASE` bit 11: the APIC is enabled.
const BASE_ENABLED: u64 = 1 << 11;
/// `IA32_APIC_BASE` bits 12 and up: the register page.
const BASE_ADDRESS: u64 = !0xfff;

const ID: usize = 0x20;
const TASK_PRIORITY: usize = 0x80;
const EOI: usize = 0xb0;
const SPURIOUS_VECTOR: usize = 0xf0;
const LVT_TIMER: usize = 0x320;
const LVT_LINT0: usize = 0x350;
const LVT_LINT1: usize = 0x360;
const LVT_ERROR: usize = 0x370;
const TIMER_INITIAL: usize = 0x380;
const TIMER_CURRENT: usize = 0x390;
const TIMER_DIVIDE: usize = 0x3e0;

/// Spurious vector register bit 8: software enable.
const APIC_ENABLED: u32 = 1 << 8;
/// LVT bit 16: masked.
const LVT_MASKED: u32 = 1 << 16;
/// LVT timer bit 17: periodic mode.
const LVT_PERIODIC: u32 = 1 << 17;
/// LVT delivery mode NMI, bits 8-10.
const LVT_NMI: u32 = 0b100 << 8;
/// LVT bit 13: active low.
const LVT_ACTIVE_LOW: u32 = 1 << 13;
/// Divide configuration 0b0011: the timer counts at the bus clock / 16.
const DIVIDE_BY_16: u32 = 0b0011;
/// MPS INTI flags bits 0-1 at 0b11: active low.
const INTI_ACTIVE_LOW: u16 = 0b11;

/// How long calibration measures the timer.
const CALIBRATION_MS: u32 = pit::MAX_WAIT_MS;

/// The register page, mapped uncached.
static REGISTERS: WriteOnce<VirtAddr> = WriteOnce::new();

/// Timer interrupts since [`init`].
static TICKS: AtomicU64 = AtomicU64::new(0);

fn register(offset: usize) -> *mut u32 {
    let base = REGISTERS.get().expect("lapic::init ran");
    (*base + offset as u64).as_mut_ptr()
}

fn read(offset: usize) -> u32 {
    // SAFETY: `init` mapped the register page, and every offset here is
    // a readable register.
    unsafe { register(offset).read_volatile() }
}

fn write(offset: usize, value: u32) {
    // SAFETY: `init` mapped the register page, and every offset here is
    // a writable register. Each CPU writes only its own APIC.
    unsafe { register(offset).write_volatile(value) }
}

/// Enables the boot CPU's local APIC and starts its timer. Returns the
/// timer's count rate in ticks per millisecond, after the divider.
///
/// # Panics
///
/// Panics if the APIC is disabled or in x2APIC mode, if its address
/// differs from the MADT's, or if called twice.
pub(crate) fn init(madt: &Madt) -> u32 {
    let base = cpu::apic_base();
    assert!(base & BASE_ENABLED != 0, "the local APIC is enabled");
    assert!(base & BASE_X2APIC == 0, "the local APIC is in xAPIC mode");
    assert_eq!(
        base & BASE_ADDRESS,
        madt.local_apic.as_u64(),
        "the local APIC is where the MADT says"
    );
    REGISTERS.init(memory::map_mmio(madt.local_apic, PAGE_SIZE));
    irq::set_handler(SPURIOUS, || {});
    irq::set_handler(TIMER, tick);

    write(TASK_PRIORITY, 0);
    write(LVT_LINT0, lint(madt, 0));
    write(LVT_LINT1, lint(madt, 1));
    write(LVT_ERROR, LVT_MASKED);
    write(LVT_TIMER, LVT_MASKED);
    write(SPURIOUS_VECTOR, APIC_ENABLED | u32::from(SPURIOUS));

    let per_ms = calibrate();
    let per_tick = u64::from(per_ms) * 1000 / HZ;
    write(LVT_TIMER, LVT_PERIODIC | u32::from(TIMER));
    write(
        TIMER_INITIAL,
        u32::try_from(per_tick).expect("one timer period fits the count"),
    );
    per_ms
}

/// The LVT entry for pin `lint`: NMI if the MADT wires it to NMI on this
/// CPU, masked otherwise. Firmware may leave LINT0 in `ExtINT` mode, passing
/// the 8259 through, so masking it keeps the PICs out.
fn lint(madt: &Madt, lint: u8) -> u32 {
    let apic_id = id();
    let acpi_id = madt
        .cpus
        .iter()
        .find(|cpu| u32::from(cpu.apic_id) == apic_id)
        .map(|cpu| cpu.acpi_id);
    let nmi = madt
        .local_apic_nmis
        .iter()
        .find(|nmi| nmi.lint == lint && (nmi.acpi_id.is_none() || nmi.acpi_id == acpi_id));
    match nmi {
        // NMIs are edge triggered whatever the flags say (SDM Vol. 3A
        // §13.5.1), so only the polarity matters.
        Some(nmi) if nmi.flags & INTI_ACTIVE_LOW == INTI_ACTIVE_LOW => LVT_NMI | LVT_ACTIVE_LOW,
        Some(_) => LVT_NMI,
        None => LVT_MASKED,
    }
}

/// Counts the timer against the PIT, in one-shot mode with the timer
/// masked, and returns ticks per millisecond.
fn calibrate() -> u32 {
    write(TIMER_DIVIDE, DIVIDE_BY_16);
    pit::wait_ms(CALIBRATION_MS, || write(TIMER_INITIAL, u32::MAX));
    let elapsed = u32::MAX - read(TIMER_CURRENT);
    write(TIMER_INITIAL, 0);
    elapsed / CALIBRATION_MS
}

/// This CPU's APIC ID.
pub(crate) fn id() -> u32 {
    read(ID) >> 24
}

/// Signals the end of the interrupt being handled, so the APIC delivers
/// the next one of the same or lower priority.
pub(crate) fn eoi() {
    write(EOI, 0);
}

/// Timer interrupts since [`init`].
pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// The timer tick count as a clock source, at [`HZ`] counts per second.
pub(crate) struct TickClock;

impl crate::time::ClockSource for TickClock {
    fn name(&self) -> &'static str {
        "local APIC ticks"
    }

    fn read(&self) -> u64 {
        ticks()
    }

    fn frequency(&self) -> u64 {
        HZ
    }
}

/// The timer handler: counts the tick, wakes the sleepers whose deadline
/// it reaches, and asks for a thread switch, which the interrupt exit
/// makes.
fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    crate::time::expire();
    percpu::current(&IrqGuard::new()).set_need_resched();
}

ktest::ktest! {
    fn local_apic_is_enabled_on_the_boot_cpu() {
        assert!(read(SPURIOUS_VECTOR) & APIC_ENABLED != 0);
        assert_eq!(read(SPURIOUS_VECTOR) & 0xff, u32::from(SPURIOUS));
        let boot = crate::acpi::madt().cpus[0];
        assert_eq!(id(), u32::from(boot.apic_id));
    }
}

ktest::ktest! {
    fn qemu_wires_lint1_to_nmi_and_masks_lint0() {
        assert_eq!(read(LVT_LINT0), LVT_MASKED);
        assert_eq!(read(LVT_LINT1) & (LVT_MASKED | 0b111 << 8), LVT_NMI);
    }
}

ktest::ktest! {
    fn timer_ticks_at_hz() {
        // QEMU's timers drift against each other under host load, so the
        // check allows a wide margin.
        const WAITS: u64 = 4;
        let start = ticks();
        for _ in 0..WAITS {
            pit::wait_ms(pit::MAX_WAIT_MS, || {});
        }
        let counted = ticks() - start;
        let ms = WAITS * u64::from(pit::MAX_WAIT_MS);
        let expected = HZ * ms / 1000;
        assert!(
            (expected / 2..=expected * 2).contains(&counted),
            "{counted} ticks in {ms} ms, expected about {expected}"
        );
    }
}
