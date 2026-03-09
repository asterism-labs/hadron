//! Legacy platform initialization (non-ACPI).
//!
//! Uses 8259 PIC for interrupts and 8254 PIT for system timer.
//! This is a minimal fallback — no HPET, no APIC, no SMP.

/// Default PIC IRQ mask: enable IRQ 0 (PIT timer), IRQ 1 (keyboard), IRQ 2 (cascade).
/// Bits set to 0 = IRQ enabled, bits set to 1 = IRQ masked.
const PIC_DEFAULT_MASK: u16 = !(1 << 0 | 1 << 1 | 1 << 2);

/// Initialize legacy interrupt controller and timer.
///
/// Remaps the PIC to vectors 32-47, enables IRQs 0/1/2, and starts PIT
/// channel 0 at ~1000 Hz for the system timer.
pub fn init() {
    crate::kinfo!("Legacy: initializing PIC + PIT (no ACPI)");

    // 1. Remap PIC to vectors 32-47 and enable needed IRQs.
    // SAFETY: Called once during early boot with interrupts disabled.
    unsafe { super::hw::pic::remap_and_enable(PIC_DEFAULT_MASK) };

    // 2. Start PIT channel 0 at ~1000 Hz for system timer.
    // SAFETY: Called once during early boot with interrupts disabled. Channel 0
    // is not in use.
    unsafe { super::hw::pit::start_periodic(1000) };

    // 3. Initialize time subsystem with PIT-based timing.
    crate::time::Time::init_pit();

    // 4. Read CMOS RTC for wall-clock epoch.
    crate::time::Time::init_rtc_epoch();

    crate::kinfo!("Legacy: PIC remapped, PIT running at 1000 Hz");
}
