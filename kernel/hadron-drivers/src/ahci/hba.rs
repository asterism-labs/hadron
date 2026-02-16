//! AHCI HBA (Host Bus Adapter) controller.
//!
//! Provides safe volatile MMIO access to the generic host control registers
//! and methods to enable AHCI mode and query capabilities.

use core::ptr;

use hadron_core::addr::VirtAddr;

use super::regs::{self, HbaCap, HbaGhc};

/// AHCI HBA controller state.
pub struct AhciHba {
    /// Virtual base address of the HBA MMIO region (ABAR).
    base: VirtAddr,
    /// Number of command slots per port (1-32).
    pub num_cmd_slots: u8,
    /// Whether the HBA supports 64-bit addressing.
    pub supports_64bit: bool,
}

impl AhciHba {
    /// Creates a new HBA handle by reading capabilities from MMIO registers.
    ///
    /// # Safety
    ///
    /// `base` must point to a valid, mapped AHCI HBA MMIO region.
    pub unsafe fn new(base: VirtAddr) -> Self {
        let cap_raw = unsafe { Self::read32_at(base, regs::HBA_CAP) };
        let cap = HbaCap::from_bits_retain(cap_raw);

        Self {
            base,
            num_cmd_slots: cap.num_cmd_slots(),
            supports_64bit: cap.contains(HbaCap::S64A),
        }
    }

    /// Enables AHCI mode and global interrupts.
    pub fn enable(&self) {
        let ghc = self.read32(regs::HBA_GHC);
        let new_ghc = ghc | HbaGhc::AE.bits() | HbaGhc::IE.bits();
        self.write32(regs::HBA_GHC, new_ghc);
    }

    /// Returns the Ports Implemented bitmask.
    #[must_use]
    pub fn ports_implemented(&self) -> u32 {
        self.read32(regs::HBA_PI)
    }

    /// Returns the AHCI version as (major, minor).
    #[must_use]
    pub fn version(&self) -> (u16, u16) {
        let vs = self.read32(regs::HBA_VS);
        ((vs >> 16) as u16, vs as u16)
    }

    /// Returns the virtual base address of a port's register block.
    #[must_use]
    pub fn port_base(&self, port: u8) -> VirtAddr {
        VirtAddr::new(
            self.base.as_u64() + regs::PORT_BASE + u64::from(port) * regs::PORT_REG_SIZE,
        )
    }

    /// Reads a 32-bit MMIO register at the given offset from the HBA base.
    #[must_use]
    pub fn read32(&self, offset: u64) -> u32 {
        // SAFETY: base is a valid mapped MMIO region, offset within HBA space.
        unsafe { Self::read32_at(self.base, offset) }
    }

    /// Writes a 32-bit MMIO register at the given offset from the HBA base.
    pub fn write32(&self, offset: u64, value: u32) {
        let addr = (self.base.as_u64() + offset) as *mut u32;
        // SAFETY: base is a valid mapped MMIO region.
        unsafe { ptr::write_volatile(addr, value) };
    }

    /// Volatile read helper.
    unsafe fn read32_at(base: VirtAddr, offset: u64) -> u32 {
        let addr = (base.as_u64() + offset) as *const u32;
        unsafe { ptr::read_volatile(addr) }
    }
}
