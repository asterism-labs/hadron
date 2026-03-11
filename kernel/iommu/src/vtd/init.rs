//! Per-DRHD VT-d unit initialization.
//!
//! For each DRHD entry from the DMAR table:
//! 1. Map MMIO registers
//! 2. Read CAP/ECAP
//! 3. Allocate + set root table
//! 4. Flush caches
//! 5. Enable translation

use hadron_core::addr::{PhysAddr, VirtAddr};

use super::VtdUnit;
use super::regs::{self, GCMD_SRTP, GCMD_TE, GCMD_WBF, GSTS_RTPS, GSTS_TES, GSTS_WBFIS, VtdRegs};
use super::tables;
use super::tlb;
use crate::DrhdEntry;
use crate::domain::DomainAllocator;

/// Maximum number of poll iterations when waiting for hardware status bits.
const POLL_TIMEOUT: u32 = 100_000;

/// Initialize all VT-d units from parsed DMAR info.
pub(crate) fn init_all_units(host_address_width: u8, drhds: &[DrhdEntry]) {
    if drhds.is_empty() {
        hadron_log::kdebug!("iommu", "VT-d: no DRHD entries, skipping");
        return;
    }

    hadron_log::kinfo!(
        "iommu",
        "VT-d: initializing {} unit(s), host address width {}",
        drhds.len(),
        host_address_width + 1
    );

    let mut units = crate::VTD_UNITS.lock();

    for (idx, drhd) in drhds.iter().enumerate() {
        match init_unit(idx, drhd) {
            Ok(unit) => {
                hadron_log::kinfo!(
                    "iommu",
                    "VT-d unit {} enabled: base={:#x}, domains={}",
                    idx,
                    drhd.register_base_address,
                    unit.domains.max_domains()
                );
                units.push(unit);
            }
            Err(msg) => {
                hadron_log::kerror!(
                    "iommu",
                    "VT-d unit {} init failed (base={:#x}): {}",
                    idx,
                    drhd.register_base_address,
                    msg
                );
            }
        }
    }
}

/// Initialize a single VT-d remapping unit.
fn init_unit(index: usize, drhd: &DrhdEntry) -> Result<VtdUnit, &'static str> {
    let reg_phys = PhysAddr::new(drhd.register_base_address);

    // Step 1: Map MMIO registers (permanent mapping, cleanup=None).
    let mmio_base = map_vtd_mmio(reg_phys)?;

    // SAFETY: mmio_base was just mapped to the VT-d register block.
    let regs = unsafe { VtdRegs::new(mmio_base) };

    // Step 2: Read CAP and ECAP.
    let ver = regs.ver();
    let cap = regs.cap();
    let ecap = regs.ecap();

    hadron_log::kdebug!(
        "iommu",
        "VT-d unit {}: ver={:#x}, CAP={:#x}, ECAP={:#x}",
        index,
        ver,
        cap,
        ecap
    );

    // Decode number of supported domains.
    let nd = regs::cap_nd(cap);
    let max_domains = DomainAllocator::decode_nd(nd);
    let domains = DomainAllocator::new(max_domains);

    // Check for supported AGAW (need at least 39-bit or 48-bit).
    let sagaw = regs::cap_sagaw(cap);
    if sagaw & 0x06 == 0 {
        return Err("no supported AGAW (need 39-bit or 48-bit)");
    }

    // Step 3: Allocate root table.
    let root_table_phys = tables::alloc_table_frame();

    // Step 4: Write root table address to RTADDR register.
    regs.set_rtaddr(root_table_phys.as_u64());

    // Step 5: Set GCMD.SRTP and poll GSTS.RTPS.
    regs.set_gcmd(GCMD_SRTP);
    if !poll_status(&regs, GSTS_RTPS, true) {
        return Err("timeout waiting for GSTS.RTPS");
    }

    // Step 6: Write-buffer flush if required (CAP.RWBF).
    if regs::cap_rwbf(cap) {
        regs.set_gcmd(GCMD_WBF);
        if !poll_status(&regs, GSTS_WBFIS, false) {
            return Err("timeout waiting for write-buffer flush");
        }
    }

    // Step 7: Global context-cache invalidation.
    tlb::invalidate_context_global(&regs);

    // Step 8: Global IOTLB invalidation.
    tlb::invalidate_iotlb_global(&regs, ecap);

    // Step 9: Check for any pending faults and clear them.
    super::fault::check_and_clear(&regs, cap);

    // Step 10: Enable translation (GCMD.TE).
    regs.set_gcmd(GCMD_TE);
    if !poll_status(&regs, GSTS_TES, true) {
        return Err("timeout waiting for GSTS.TES (translation enable)");
    }

    Ok(VtdUnit {
        index,
        mmio_base,
        cap,
        ecap,
        root_table_phys,
        domains,
        context_tables: alloc::vec![None; tables::ROOT_TABLE_ENTRIES],
    })
}

/// Map VT-d MMIO registers into kernel virtual address space.
fn map_vtd_mmio(phys: PhysAddr) -> Result<VirtAddr, &'static str> {
    // Use HHDM direct mapping for MMIO (no VMM lock nesting issues).
    // VT-d registers are within the first 4 GiB, always covered by HHDM.
    Ok(hadron_mm::hhdm::phys_to_virt(phys))
}

/// Poll the GSTS register until the expected bit is set (or cleared).
///
/// Returns `true` on success, `false` on timeout.
fn poll_status(regs: &VtdRegs, bit: u32, expect_set: bool) -> bool {
    for _ in 0..POLL_TIMEOUT {
        let gsts = regs.gsts();
        let is_set = gsts & bit != 0;
        if is_set == expect_set {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}
