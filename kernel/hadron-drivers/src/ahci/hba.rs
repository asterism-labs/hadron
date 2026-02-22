//! AHCI HBA (Host Bus Adapter) controller.
//!
//! Provides safe volatile MMIO access to the generic host control registers
//! and methods to enable AHCI mode and query capabilities.

use hadron_kernel::addr::VirtAddr;

use super::regs::{self, AhciHbaRegs, HbaCap, HbaGhc};

/// AHCI HBA controller state.
pub struct AhciHba {
    /// Typed register block for the HBA MMIO region (ABAR).
    regs: AhciHbaRegs,
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
        // SAFETY: Caller guarantees base is a valid AHCI HBA MMIO region.
        let regs = unsafe { AhciHbaRegs::new(base) };
        let cap = regs.cap();

        Self {
            regs,
            num_cmd_slots: cap.num_cmd_slots(),
            supports_64bit: cap.contains(HbaCap::S64A),
        }
    }

    /// Enables AHCI mode and global interrupts.
    pub fn enable(&self) {
        let ghc = self.regs.ghc();
        self.regs.set_ghc(ghc | HbaGhc::AE | HbaGhc::IE);
    }

    /// Returns the Ports Implemented bitmask.
    #[must_use]
    pub fn ports_implemented(&self) -> u32 {
        self.regs.pi()
    }

    /// Returns the AHCI version as (major, minor).
    #[must_use]
    pub fn version(&self) -> (u16, u16) {
        let vs = self.regs.vs();
        ((vs >> 16) as u16, vs as u16)
    }

    /// Returns the virtual base address of a port's register block.
    #[must_use]
    pub fn port_base(&self, port: u8) -> VirtAddr {
        VirtAddr::new(
            self.regs.base().as_u64() + regs::PORT_BASE + u64::from(port) * regs::PORT_REG_SIZE,
        )
    }
}
