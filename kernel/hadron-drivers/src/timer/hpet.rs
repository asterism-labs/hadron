//! HPET (High Precision Event Timer) driver.
//!
//! Provides MMIO-based access to the HPET for high-resolution timing
//! and calibration.

use hadron_kernel::addr::VirtAddr;
use hadron_kernel::driver_api::ClockSource;
use hadron_mmio::register_block;

register_block! {
    /// HPET timer MMIO registers.
    HpetRegs {
        /// General Capabilities and ID (read-only).
        [0x000; u64; ro] capabilities,
        /// General Configuration.
        [0x010; u64; rw] configuration,
        /// Main Counter Value.
        [0x0F0; u64; rw] main_counter,
    }
}

/// Femtoseconds per second.
const FS_PER_SECOND: u64 = 1_000_000_000_000_000;

/// HPET timer driver.
pub struct Hpet {
    regs: HpetRegs,
    /// Counter period in femtoseconds (from capabilities register bits 63:32).
    period_fs: u64,
}

impl Hpet {
    /// Creates a new HPET driver and reads the counter period.
    ///
    /// # Safety
    ///
    /// `virt_base` must be a valid mapping of the HPET MMIO region.
    pub unsafe fn new(virt_base: VirtAddr) -> Self {
        // SAFETY: Caller guarantees virt_base is a valid HPET MMIO region.
        let regs = unsafe { HpetRegs::new(virt_base) };
        let caps = regs.capabilities();
        let period_fs = caps >> 32;
        Self { regs, period_fs }
    }

    /// Returns the counter period in femtoseconds per tick.
    pub fn period_fs(&self) -> u64 {
        self.period_fs
    }

    /// Returns the HPET frequency in Hz.
    pub fn frequency_hz(&self) -> u64 {
        if self.period_fs == 0 {
            return 0;
        }
        FS_PER_SECOND / self.period_fs
    }

    /// Returns the number of timer comparators from the capabilities register.
    pub fn num_comparators(&self) -> u8 {
        let caps = self.regs.capabilities();
        (((caps >> 8) & 0x1F) + 1) as u8
    }

    /// Enables the HPET main counter.
    pub fn enable(&self) {
        let mut config = self.regs.configuration();
        config |= 1; // ENABLE_CNF bit
        self.regs.set_configuration(config);
    }

    /// Disables the HPET main counter.
    pub fn disable(&self) {
        let mut config = self.regs.configuration();
        config &= !1;
        self.regs.set_configuration(config);
    }

    /// Reads the HPET main counter value.
    pub fn read_counter(&self) -> u64 {
        self.regs.main_counter()
    }

    /// Busy-waits for approximately `ms` milliseconds using the HPET counter.
    pub fn busy_wait_ms(&self, ms: u32) {
        let ticks_needed = (u64::from(ms) * FS_PER_SECOND) / (1000 * self.period_fs);
        let start = self.read_counter();
        while self.read_counter().wrapping_sub(start) < ticks_needed {
            core::hint::spin_loop();
        }
    }

    /// Returns the HPET virtual base address.
    pub fn base(&self) -> VirtAddr {
        self.regs.base()
    }
}

impl ClockSource for Hpet {
    fn read_nanos(&self) -> u64 {
        let counter = self.read_counter();
        // ticks * period_fs / 1_000_000 = nanoseconds
        (counter as u128 * self.period_fs as u128 / 1_000_000) as u64
    }
}
