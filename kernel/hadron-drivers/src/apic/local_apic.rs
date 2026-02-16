//! Local APIC (Advanced Programmable Interrupt Controller) driver.
//!
//! Provides MMIO-based access to the Local APIC for interrupt management,
//! timer configuration, and inter-processor interrupts.

use hadron_core::addr::VirtAddr;

// Register offsets from LAPIC base.
const REG_ID: u32 = 0x020;
const REG_VERSION: u32 = 0x030;
const REG_TPR: u32 = 0x080;
const REG_EOI: u32 = 0x0B0;
const REG_SVR: u32 = 0x0F0;
const REG_ICR_LOW: u32 = 0x300;
const REG_ICR_HIGH: u32 = 0x310;
const REG_LVT_TIMER: u32 = 0x320;
const REG_TIMER_INITIAL: u32 = 0x380;
const REG_TIMER_CURRENT: u32 = 0x390;
const REG_TIMER_DIVIDE: u32 = 0x3E0;

/// SVR enable bit.
const SVR_ENABLE: u32 = 1 << 8;

/// LVT timer mode bits.
const TIMER_PERIODIC: u32 = 1 << 17;
const TIMER_MASKED: u32 = 1 << 16;

/// MSR address for APIC base.
pub const IA32_APIC_BASE_MSR: u32 = 0x1B;

/// Local APIC driver using MMIO register access.
pub struct LocalApic {
    base: VirtAddr,
}

impl LocalApic {
    /// Creates a new Local APIC driver.
    ///
    /// # Safety
    ///
    /// `virt_base` must be a valid mapping of the LAPIC MMIO region (at least 4 KiB).
    pub unsafe fn new(virt_base: VirtAddr) -> Self {
        Self { base: virt_base }
    }

    /// Returns the APIC ID of this processor.
    pub fn id(&self) -> u8 {
        ((self.read_reg(REG_ID) >> 24) & 0xFF) as u8
    }

    /// Returns the APIC version.
    pub fn version(&self) -> u32 {
        self.read_reg(REG_VERSION)
    }

    /// Enables the Local APIC with the given spurious interrupt vector.
    pub fn enable(&self, spurious_vector: u8) {
        let svr = SVR_ENABLE | u32::from(spurious_vector);
        self.write_reg(REG_SVR, svr);
    }

    /// Sends an End-of-Interrupt signal.
    pub fn eoi(&self) {
        self.write_reg(REG_EOI, 0);
    }

    /// Sets the Task Priority Register (0 = accept all interrupts).
    pub fn set_tpr(&self, priority: u8) {
        self.write_reg(REG_TPR, u32::from(priority));
    }

    /// Starts the LAPIC timer in periodic mode.
    pub fn start_timer_periodic(&self, vector: u8, initial_count: u32, divide: u8) {
        self.write_reg(REG_TIMER_DIVIDE, divide_config(divide));
        self.write_reg(REG_LVT_TIMER, TIMER_PERIODIC | u32::from(vector));
        self.write_reg(REG_TIMER_INITIAL, initial_count);
    }

    /// Starts the LAPIC timer in one-shot mode.
    pub fn start_timer_oneshot(&self, vector: u8, initial_count: u32, divide: u8) {
        self.write_reg(REG_TIMER_DIVIDE, divide_config(divide));
        self.write_reg(REG_LVT_TIMER, u32::from(vector));
        self.write_reg(REG_TIMER_INITIAL, initial_count);
    }

    /// Stops the LAPIC timer by masking it.
    pub fn stop_timer(&self) {
        self.write_reg(REG_LVT_TIMER, TIMER_MASKED);
    }

    /// Returns the current timer count.
    pub fn timer_current_count(&self) -> u32 {
        self.read_reg(REG_TIMER_CURRENT)
    }

    /// Sends an IPI (Inter-Processor Interrupt) to a target CPU.
    ///
    /// # Safety
    ///
    /// The caller must ensure the target APIC ID is valid and the vector
    /// is appropriately configured.
    pub unsafe fn send_ipi(&self, target_apic_id: u8, vector: u8) {
        // Write destination APIC ID to high dword.
        self.write_reg(REG_ICR_HIGH, u32::from(target_apic_id) << 24);
        // Write vector to low dword (fixed delivery, physical destination).
        self.write_reg(REG_ICR_LOW, u32::from(vector));
        // Wait for delivery.
        while self.read_reg(REG_ICR_LOW) & (1 << 12) != 0 {
            core::hint::spin_loop();
        }
    }

    #[inline]
    fn read_reg(&self, offset: u32) -> u32 {
        // SAFETY: The caller of `LocalApic::new` guarantees that `self.base` points to
        // a valid LAPIC MMIO region. All register offsets used are within the 4 KiB page.
        unsafe {
            let ptr = (self.base.as_u64() + u64::from(offset)) as *const u32;
            core::ptr::read_volatile(ptr)
        }
    }

    #[inline]
    fn write_reg(&self, offset: u32, value: u32) {
        // SAFETY: The caller of `LocalApic::new` guarantees that `self.base` points to
        // a valid LAPIC MMIO region. All register offsets used are within the 4 KiB page.
        unsafe {
            let ptr = (self.base.as_u64() + u64::from(offset)) as *mut u32;
            core::ptr::write_volatile(ptr, value);
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
