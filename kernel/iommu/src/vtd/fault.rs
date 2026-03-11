//! VT-d fault status register reading and logging.
//!
//! Poll-based only — interrupt-driven faults deferred to Phase 4c.
//!
//! Reference: Intel VT-d Specification, Section 10.4.9-10.4.10.

use super::regs::{self, FSTS_PFO, FSTS_PPF, VtdRegs};

/// Check for pending faults and log/clear them.
///
/// Called during init and can be called periodically for diagnostics.
pub(crate) fn check_and_clear(regs: &VtdRegs, cap: u64) {
    let fsts = regs.fsts();

    if fsts & FSTS_PPF == 0 && fsts & FSTS_PFO == 0 {
        return; // No faults pending
    }

    if fsts & FSTS_PFO != 0 {
        hadron_log::kwarn!("iommu", "VT-d: fault recording overflow detected");
    }

    // Read fault recording registers.
    let fro = regs::cap_fro(cap);
    let nfr = regs::cap_nfr(cap);
    let base = regs.base().as_u64();

    for i in 0..u16::from(nfr) {
        let fr_offset = u64::from(fro) * 16 + u64::from(i) * 16;
        let fr_addr = base + fr_offset;

        // SAFETY: The VT-d MMIO region includes fault recording registers.
        let (fr_low, fr_high) = unsafe {
            let lo = core::ptr::read_volatile(fr_addr as *const u64);
            let hi = core::ptr::read_volatile((fr_addr + 8) as *const u64);
            (lo, hi)
        };

        // Bit 127 (bit 63 of high) = Fault (F) — set if this record is valid.
        if fr_high & (1u64 << 63) == 0 {
            continue;
        }

        let fault_addr = fr_low;
        let source_id = ((fr_high >> 0) & 0xFFFF) as u16;
        let reason = ((fr_high >> 32) & 0xFF) as u8;
        let is_write = fr_high & (1 << 29) != 0;

        hadron_log::kerror!(
            "iommu",
            "VT-d fault: addr={:#x}, source={:#06x}, reason={}, type={}",
            fault_addr,
            source_id,
            reason,
            if is_write { "write" } else { "read" }
        );

        // Clear the fault record by writing 1 to the F bit.
        // SAFETY: Writing to the fault recording register to clear it.
        unsafe {
            core::ptr::write_volatile((fr_addr + 8) as *mut u64, 1u64 << 63);
        }
    }

    // Clear PPF and PFO by writing 1 to them.
    regs.set_fsts(FSTS_PPF | FSTS_PFO);
}
