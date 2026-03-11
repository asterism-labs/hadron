//! IOTLB and context-cache invalidation.
//!
//! IOTLB registers are at a variable offset: `ECAP.IRO * 16 + 8`.
//! Context-cache invalidation uses the fixed `CCMD` register.
//!
//! Reference: Intel VT-d Specification, Sections 10.4.4-10.4.5.

use hadron_core::addr::VirtAddr;

use crate::domain::DomainId;

use super::regs::{self, CCMD_CIRG_DOMAIN, CCMD_CIRG_GLOBAL, CCMD_ICC, VtdRegs};

/// Maximum poll iterations for invalidation completion.
const INVALIDATION_TIMEOUT: u32 = 100_000;

/// Perform a global context-cache invalidation.
///
/// Writes `ICC | CIRG_GLOBAL` to the CCMD register and polls until ICC clears.
pub(crate) fn invalidate_context_global(regs: &VtdRegs) {
    regs.set_ccmd(CCMD_ICC | CCMD_CIRG_GLOBAL);

    for _ in 0..INVALIDATION_TIMEOUT {
        if regs.ccmd() & CCMD_ICC == 0 {
            return;
        }
        core::hint::spin_loop();
    }

    hadron_log::kwarn!("iommu", "VT-d: context-cache global invalidation timeout");
}

/// IOTLB invalidation request granularity — global (bits 62:60 = 001).
const IOTLB_IIRG_GLOBAL: u64 = 1 << 60;
/// IOTLB invalidation bit (bit 63): set to start, poll until clear.
const IOTLB_IVT: u64 = 1 << 63;
/// Drain reads (bit 49).
const IOTLB_DR: u64 = 1 << 49;
/// Drain writes (bit 48).
const IOTLB_DW: u64 = 1 << 48;

/// Perform a global IOTLB invalidation.
///
/// The IOTLB register is at offset `ECAP.IRO * 16 + 8` from the MMIO base.
pub(crate) fn invalidate_iotlb_global(regs: &VtdRegs, ecap: u64) {
    let iro = regs::ecap_iro(ecap);
    let iotlb_offset = u64::from(iro) * 16 + 8;
    let iotlb_addr = regs.base().as_u64() + iotlb_offset;

    let cmd = IOTLB_IVT | IOTLB_IIRG_GLOBAL | IOTLB_DR | IOTLB_DW;

    // SAFETY: The VT-d MMIO region was mapped during init and includes the
    // IOTLB register at this offset. Writing to MMIO is volatile.
    unsafe {
        let ptr = iotlb_addr as *mut u64;
        core::ptr::write_volatile(ptr, cmd);
    }

    // Poll until IVT clears.
    for _ in 0..INVALIDATION_TIMEOUT {
        // SAFETY: Reading the IOTLB register via volatile.
        let val = unsafe { core::ptr::read_volatile(iotlb_addr as *const u64) };
        if val & IOTLB_IVT == 0 {
            return;
        }
        core::hint::spin_loop();
    }

    hadron_log::kwarn!("iommu", "VT-d: IOTLB global invalidation timeout");
}

/// IOTLB invalidation request granularity — domain-selective (bits 61:60 = 10).
const IOTLB_IIRG_DOMAIN: u64 = 2 << 60;

/// Perform a domain-selective context-cache invalidation.
///
/// Invalidates all context entries matching `domain_id`. The domain ID is
/// placed in CCMD bits 23:8.
pub(crate) fn invalidate_context_domain(regs: &VtdRegs, domain_id: DomainId) {
    let did = u64::from(domain_id.as_u16()) << 8;
    regs.set_ccmd(CCMD_ICC | CCMD_CIRG_DOMAIN | did);

    for _ in 0..INVALIDATION_TIMEOUT {
        if regs.ccmd() & CCMD_ICC == 0 {
            return;
        }
        core::hint::spin_loop();
    }

    hadron_log::kwarn!("iommu", "VT-d: context-cache domain invalidation timeout");
}

/// Perform a domain-selective IOTLB invalidation.
///
/// Invalidates all IOTLB entries for `domain_id`. The domain ID is placed
/// in the IOTLB register bits 47:32.
pub(crate) fn invalidate_iotlb_domain(mmio_base: VirtAddr, ecap: u64, domain_id: DomainId) {
    let iro = regs::ecap_iro(ecap);
    let iotlb_offset = u64::from(iro) * 16 + 8;
    let iotlb_addr = mmio_base.as_u64() + iotlb_offset;

    let did = u64::from(domain_id.as_u16()) << 32;
    let cmd = IOTLB_IVT | IOTLB_IIRG_DOMAIN | IOTLB_DR | IOTLB_DW | did;

    // SAFETY: The VT-d MMIO region was mapped during init and includes the
    // IOTLB register at this offset. Writing to MMIO is volatile.
    unsafe {
        let ptr = iotlb_addr as *mut u64;
        core::ptr::write_volatile(ptr, cmd);
    }

    // Poll until IVT clears.
    for _ in 0..INVALIDATION_TIMEOUT {
        // SAFETY: Reading the IOTLB register via volatile.
        let val = unsafe { core::ptr::read_volatile(iotlb_addr as *const u64) };
        if val & IOTLB_IVT == 0 {
            return;
        }
        core::hint::spin_loop();
    }

    hadron_log::kwarn!("iommu", "VT-d: IOTLB domain invalidation timeout");
}
