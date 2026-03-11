//! Intel VT-d MMIO register definitions.
//!
//! Uses the `register_block!` macro for typed access to the fixed-offset
//! registers. IOTLB registers live at a variable offset determined by
//! `ECAP.IRO` and are accessed separately in [`super::tlb`].
//!
//! Reference: Intel VT-d Specification, Section 10 — Register Descriptions.

use hadron_core::addr::VirtAddr;
use hadron_mmio::register_block;

register_block! {
    /// Intel VT-d remapping unit MMIO registers (fixed offsets).
    pub VtdRegs {
        /// Version Register.
        [0x00; u32; ro] ver,
        /// Capability Register.
        [0x08; u64; ro] cap,
        /// Extended Capability Register.
        [0x10; u64; ro] ecap,
        /// Global Command Register.
        [0x18; u32; wo] gcmd,
        /// Global Status Register.
        [0x1C; u32; ro] gsts,
        /// Root Table Address Register.
        [0x20; u64; rw] rtaddr,
        /// Context Command Register.
        [0x28; u64; rw] ccmd,
        /// Fault Status Register.
        [0x34; u32; rw] fsts,
        /// Fault Event Control Register.
        [0x38; u32; rw] fectl,
        /// Fault Event Data Register.
        [0x3C; u32; rw] fedata,
        /// Fault Event Address Register.
        [0x40; u32; rw] feaddr,
        /// Fault Event Upper Address Register.
        [0x44; u32; rw] feuaddr,
    }
}

// ── Capability register (CAP) bit field helpers ──────────────────────────

/// Number of domains supported (CAP bits 2:0).
#[must_use]
pub fn cap_nd(cap: u64) -> u8 {
    (cap & 0x07) as u8
}

/// Required Write Buffer Flushing (CAP bit 4).
#[must_use]
pub fn cap_rwbf(cap: u64) -> bool {
    cap & (1 << 4) != 0
}

/// Supported Adjusted Guest Address Widths (CAP bits 12:8).
///
/// Returns a bitmask where bit N means (N+2)-level page table is supported:
/// - bit 0 (AGAW=30): 3-level (not used in practice)
/// - bit 1 (AGAW=39): 3-level, 512 GiB address space
/// - bit 2 (AGAW=48): 4-level, 256 TiB address space
#[must_use]
pub fn cap_sagaw(cap: u64) -> u8 {
    ((cap >> 8) & 0x1F) as u8
}

/// Fault Recording Register offset (CAP bits 33:24), in 16-byte units.
#[must_use]
pub fn cap_fro(cap: u64) -> u16 {
    ((cap >> 24) & 0x3FF) as u16
}

/// Number of Fault Recording Registers supported (CAP bits 47:40) + 1.
#[must_use]
pub fn cap_nfr(cap: u64) -> u8 {
    (((cap >> 40) & 0xFF) + 1) as u8
}

// ── Extended Capability register (ECAP) bit field helpers ────────────────

/// IOTLB Register Offset (ECAP bits 17:8), in 16-byte units.
#[must_use]
pub fn ecap_iro(ecap: u64) -> u16 {
    ((ecap >> 8) & 0x3FF) as u16
}

/// Page Selective Invalidation support (ECAP bit 0).
///
/// Not present in QEMU's minimal VT-d emulation — we use domain or global
/// invalidation as fallback.
#[must_use]
#[allow(dead_code)] // Phase 4b: used for optimal IOTLB invalidation
pub fn ecap_psi(ecap: u64) -> bool {
    ecap & 1 != 0
}

// ── Global Command/Status register bits ──────────────────────────────────

/// Translation Enable (GCMD bit 31).
pub const GCMD_TE: u32 = 1 << 31;
/// Set Root Table Pointer (GCMD bit 30).
pub const GCMD_SRTP: u32 = 1 << 30;
/// Write Buffer Flush (GCMD bit 27).
pub const GCMD_WBF: u32 = 1 << 27;

/// Translation Enable Status (GSTS bit 31).
pub const GSTS_TES: u32 = 1 << 31;
/// Root Table Pointer Status (GSTS bit 30).
pub const GSTS_RTPS: u32 = 1 << 30;
/// Write Buffer Flush Status (GSTS bit 27).
pub const GSTS_WBFIS: u32 = 1 << 27;

// ── Context Command register bits ────────────────────────────────────────

/// Invalidate Context Cache (CCMD bit 63).
pub const CCMD_ICC: u64 = 1 << 63;
/// Context Invalidation Request Granularity — Global (CCMD bits 62:61 = 01).
pub const CCMD_CIRG_GLOBAL: u64 = 1 << 61;
/// Context Invalidation Request Granularity — Domain (CCMD bits 62:61 = 10).
#[allow(dead_code)] // Phase 4b: used for domain-selective invalidation
pub const CCMD_CIRG_DOMAIN: u64 = 2 << 61;

// ── Fault Status register bits ───────────────────────────────────────────

/// Primary Pending Fault (FSTS bit 0).
pub const FSTS_PPF: u32 = 1;
/// Primary Fault Overflow (FSTS bit 1).
pub const FSTS_PFO: u32 = 1 << 1;
