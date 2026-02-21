//! AHCI HBA register offsets and bitflags.
//!
//! Defines the memory-mapped register layout of an AHCI Host Bus Adapter,
//! including generic host control registers and per-port register blocks.

use bitflags::bitflags;
use hadron_kernel::addr::VirtAddr;
use hadron_mmio::register_block;

// ---------------------------------------------------------------------------
// Generic Host Control register block
// ---------------------------------------------------------------------------

register_block! {
    /// AHCI HBA generic host control registers.
    pub AhciHbaRegs {
        /// Host Capabilities (read-only).
        [0x00; u32; ro] cap => HbaCap,
        /// Global Host Control.
        [0x04; u32; rw] ghc => HbaGhc,
        /// Interrupt Status.
        [0x08; u32; rw] is,
        /// Ports Implemented (read-only).
        [0x0C; u32; ro] pi,
        /// AHCI Version (read-only).
        [0x10; u32; ro] vs,
    }
}

// ---------------------------------------------------------------------------
// Per-port register block (base = hba_base + 0x100 + port * 0x80)
// ---------------------------------------------------------------------------

/// Port register block size.
pub const PORT_REG_SIZE: u64 = 0x80;
/// Base offset for port 0.
pub const PORT_BASE: u64 = 0x100;

register_block! {
    /// AHCI per-port registers.
    pub AhciPortRegs {
        /// Command List Base Address (low 32 bits).
        [0x00; u32; rw] clb,
        /// Command List Base Address (high 32 bits).
        [0x04; u32; rw] clbu,
        /// FIS Base Address (low 32 bits).
        [0x08; u32; rw] fb,
        /// FIS Base Address (high 32 bits).
        [0x0C; u32; rw] fbu,
        /// Interrupt Status.
        [0x10; u32; rw] is => PortIs,
        /// Interrupt Enable.
        [0x14; u32; rw] ie => PortIe,
        /// Command and Status.
        [0x18; u32; rw] cmd => PortCmd,
        /// Task File Data (read-only).
        [0x20; u32; ro] tfd,
        /// Signature (read-only).
        [0x24; u32; ro] sig,
        /// SATA Status (read-only).
        [0x28; u32; ro] ssts,
        /// SATA Control.
        [0x2C; u32; rw] sctl,
        /// SATA Error.
        [0x30; u32; rw] serr,
        /// Command Issue.
        [0x38; u32; rw] ci,
    }
}

// ---------------------------------------------------------------------------
// Bitflags
// ---------------------------------------------------------------------------

bitflags! {
    /// HBA Capabilities (CAP) register flags.
    #[derive(Debug, Clone, Copy)]
    pub struct HbaCap: u32 {
        /// Supports 64-bit addressing (S64A).
        const S64A = 1 << 31;
        /// Number of command slots (bits 12:8), accessed via helper.
        const _ = !0;
    }
}

impl HbaCap {
    /// Returns the number of command slots (1-32).
    #[must_use]
    pub const fn num_cmd_slots(self) -> u8 {
        (((self.bits() >> 8) & 0x1F) + 1) as u8
    }
}

bitflags! {
    /// Global Host Control (GHC) register flags.
    #[derive(Debug, Clone, Copy)]
    pub struct HbaGhc: u32 {
        /// AHCI Enable (AE).
        const AE = 1 << 31;
        /// Interrupt Enable (IE).
        const IE = 1 << 1;
        /// HBA Reset (HR).
        const HR = 1 << 0;
    }
}

bitflags! {
    /// Port Command and Status (PxCMD) register flags.
    #[derive(Debug, Clone, Copy)]
    pub struct PortCmd: u32 {
        /// Start (ST) — enables command processing.
        const ST = 1 << 0;
        /// FIS Receive Enable (FRE).
        const FRE = 1 << 4;
        /// FIS Receive Running (FR).
        const FR = 1 << 14;
        /// Command List Running (CR).
        const CR = 1 << 15;
    }
}

bitflags! {
    /// Port Interrupt Status (PxIS) register flags.
    #[derive(Debug, Clone, Copy)]
    pub struct PortIs: u32 {
        /// Device to Host Register FIS Interrupt (DHRS).
        const DHRS = 1 << 0;
        /// PIO Setup FIS Interrupt (PSS).
        const PSS = 1 << 1;
        /// DMA Setup FIS Interrupt (DSS).
        const DSS = 1 << 2;
        /// Set Device Bits Interrupt (SDBS).
        const SDBS = 1 << 3;
        /// Task File Error Status (TFES).
        const TFES = 1 << 30;
    }
}

bitflags! {
    /// Port Interrupt Enable (PxIE) register flags.
    #[derive(Debug, Clone, Copy)]
    pub struct PortIe: u32 {
        /// Device to Host Register FIS Interrupt Enable.
        const DHRE = 1 << 0;
        /// PIO Setup FIS Interrupt Enable.
        const PSE = 1 << 1;
        /// DMA Setup FIS Interrupt Enable.
        const DSE = 1 << 2;
        /// Set Device Bits Interrupt Enable.
        const SDBE = 1 << 3;
        /// Task File Error Enable.
        const TFEE = 1 << 30;
    }
}

// ---------------------------------------------------------------------------
// ATA constants
// ---------------------------------------------------------------------------

/// ATA IDENTIFY DEVICE command.
pub const ATA_CMD_IDENTIFY: u8 = 0xEC;
/// ATA READ DMA EXT command (48-bit LBA).
pub const ATA_CMD_READ_DMA_EX: u8 = 0x25;

// ---------------------------------------------------------------------------
// FIS types
// ---------------------------------------------------------------------------

/// FIS type: Register — Host to Device.
pub const FIS_TYPE_REG_H2D: u8 = 0x27;

// ---------------------------------------------------------------------------
// Device signatures
// ---------------------------------------------------------------------------

/// SATA device signature for standard ATA drives.
pub const SATA_SIG_ATA: u32 = 0x0000_0101;

// ---------------------------------------------------------------------------
// SStatus helpers
// ---------------------------------------------------------------------------

/// Extracts DET (Device Detection) field from SStatus (bits 3:0).
#[must_use]
pub const fn ssts_det(ssts: u32) -> u8 {
    (ssts & 0x0F) as u8
}

/// Extracts IPM (Interface Power Management) field from SStatus (bits 11:8).
#[must_use]
pub const fn ssts_ipm(ssts: u32) -> u8 {
    ((ssts >> 8) & 0x0F) as u8
}

/// DET value indicating device present and Phy communication established.
pub const SSTS_DET_PRESENT: u8 = 3;
/// IPM value indicating interface in active state.
pub const SSTS_IPM_ACTIVE: u8 = 1;
