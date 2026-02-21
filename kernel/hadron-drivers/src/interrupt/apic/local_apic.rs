//! Local APIC (Advanced Programmable Interrupt Controller) driver.
//!
//! Provides MMIO-based access to the Local APIC for interrupt management,
//! timer configuration, and inter-processor interrupts.

use hadron_kernel::addr::VirtAddr;
use hadron_mmio::register_block;

register_block! {
    /// Local APIC MMIO registers.
    LocalApicRegs {
        /// APIC ID (read-only).
        [0x020; u32; ro] id,
        /// APIC Version (read-only).
        [0x030; u32; ro] version,
        /// Task Priority Register.
        [0x080; u32; rw] tpr,
        /// End of Interrupt.
        [0x0B0; u32; wo] eoi,
        /// Spurious Interrupt Vector Register.
        [0x0F0; u32; rw] svr,
        /// Interrupt Command Register (low 32 bits).
        [0x300; u32; rw] icr_low,
        /// Interrupt Command Register (high 32 bits).
        [0x310; u32; rw] icr_high,
        /// LVT Timer Register.
        [0x320; u32; rw] lvt_timer,
        /// Timer Initial Count.
        [0x380; u32; rw] timer_initial,
        /// Timer Current Count (read-only).
        [0x390; u32; ro] timer_current,
        /// Timer Divide Configuration.
        [0x3E0; u32; rw] timer_divide,
    }
}

/// SVR enable bit.
const SVR_ENABLE: u32 = 1 << 8;

/// LVT timer mode bits.
const TIMER_PERIODIC: u32 = 1 << 17;
const TIMER_MASKED: u32 = 1 << 16;

/// MSR address for APIC base.
pub const IA32_APIC_BASE_MSR: u32 = 0x1B;

/// Local APIC driver using MMIO register access.
pub struct LocalApic {
    regs: LocalApicRegs,
}

impl LocalApic {
    /// Creates a new Local APIC driver.
    ///
    /// # Safety
    ///
    /// `virt_base` must be a valid mapping of the LAPIC MMIO region (at least 4 KiB).
    pub unsafe fn new(virt_base: VirtAddr) -> Self {
        // SAFETY: Caller guarantees virt_base is a valid LAPIC MMIO region.
        Self {
            regs: unsafe { LocalApicRegs::new(virt_base) },
        }
    }

    /// Returns the APIC ID of this processor.
    pub fn id(&self) -> u8 {
        ((self.regs.id() >> 24) & 0xFF) as u8
    }

    /// Returns the APIC version.
    pub fn version(&self) -> u32 {
        self.regs.version()
    }

    /// Enables the Local APIC with the given spurious interrupt vector.
    pub fn enable(&self, spurious_vector: u8) {
        let svr = SVR_ENABLE | u32::from(spurious_vector);
        self.regs.set_svr(svr);
    }

    /// Sends an End-of-Interrupt signal.
    pub fn eoi(&self) {
        self.regs.set_eoi(0);
    }

    /// Sets the Task Priority Register (0 = accept all interrupts).
    pub fn set_tpr(&self, priority: u8) {
        self.regs.set_tpr(u32::from(priority));
    }

    /// Starts the LAPIC timer in periodic mode.
    pub fn start_timer_periodic(&self, vector: u8, initial_count: u32, divide: u8) {
        self.regs.set_timer_divide(divide_config(divide));
        self.regs.set_lvt_timer(TIMER_PERIODIC | u32::from(vector));
        self.regs.set_timer_initial(initial_count);
    }

    /// Starts the LAPIC timer in one-shot mode.
    pub fn start_timer_oneshot(&self, vector: u8, initial_count: u32, divide: u8) {
        self.regs.set_timer_divide(divide_config(divide));
        self.regs.set_lvt_timer(u32::from(vector));
        self.regs.set_timer_initial(initial_count);
    }

    /// Stops the LAPIC timer by masking it.
    pub fn stop_timer(&self) {
        self.regs.set_lvt_timer(TIMER_MASKED);
    }

    /// Returns the current timer count.
    pub fn timer_current_count(&self) -> u32 {
        self.regs.timer_current()
    }

    /// Sends an IPI (Inter-Processor Interrupt) to a target CPU.
    ///
    /// # Safety
    ///
    /// The caller must ensure the target APIC ID is valid and the vector
    /// is appropriately configured.
    pub unsafe fn send_ipi(&self, target_apic_id: u8, vector: u8) {
        // Write destination APIC ID to high dword.
        self.regs.set_icr_high(u32::from(target_apic_id) << 24);
        // Write vector to low dword (fixed delivery, physical destination).
        self.regs.set_icr_low(u32::from(vector));
        // Wait for delivery.
        while self.regs.icr_low() & (1 << 12) != 0 {
            core::hint::spin_loop();
        }
    }
}

/// Converts a power-of-2 divide value to the LAPIC timer divide config register encoding.
fn divide_config(divide: u8) -> u32 {
    match divide {
        1 => 0b1011,
        2 => 0b0000,
        4 => 0b0001,
        8 => 0b0010,
        16 => 0b0011,
        32 => 0b1000,
        64 => 0b1001,
        128 => 0b1010,
        _ => 0b0011, // Default to divide by 16
    }
}
