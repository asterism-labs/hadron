//! Intel VT-d (DMA Remapping) driver.
//!
//! Implements the [`IommuHardware`](crate::hw::IommuHardware) trait for Intel
//! VT-d remapping hardware. Each DRHD entry from the ACPI DMAR table corresponds
//! to one [`VtdUnit`], which owns the MMIO register mapping, root/context tables,
//! and a domain ID allocator.

pub mod fault;
pub mod init;
pub mod regs;
pub mod slpt;
pub mod tables;
pub mod tlb;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use hadron_core::addr::{PhysAddr, VirtAddr};

use crate::domain::{DomainAllocator, DomainId};
use crate::hw::{DmaPermission, IommuError, IommuHardware, PciBdf};

/// A single VT-d remapping hardware unit.
///
/// Each unit corresponds to one DRHD entry in the DMAR table and manages
/// its own register set, root table, and domain allocator.
pub struct VtdUnit {
    /// Index of this unit (for logging).
    pub(crate) index: usize,
    /// MMIO virtual base address of the VT-d register block.
    pub(crate) mmio_base: VirtAddr,
    /// Capability register value (cached at init time).
    pub(crate) cap: u64,
    /// Extended capability register value (cached at init time).
    pub(crate) ecap: u64,
    /// Physical address of the root table (4 KiB aligned, zeroed).
    pub(crate) root_table_phys: PhysAddr,
    /// Domain ID allocator for this unit.
    pub(crate) domains: DomainAllocator,
    /// Context tables allocated for this unit (indexed by bus number).
    pub(crate) context_tables: Vec<Option<PhysAddr>>,
    /// Per-domain second-level page tables (keyed by domain ID).
    pub(crate) slpts: BTreeMap<u16, slpt::Slpt>,
}

impl IommuHardware for VtdUnit {
    fn alloc_domain(&mut self) -> Result<DomainId, IommuError> {
        let id = self.domains.alloc()?;
        let agaw = select_agaw(self.cap);
        let slpt = slpt::Slpt::new(agaw);
        self.slpts.insert(id.as_u16(), slpt);
        Ok(id)
    }

    fn free_domain(&mut self, domain: DomainId) -> Result<(), IommuError> {
        self.slpts.remove(&domain.as_u16()); // Drop frees all table pages
        self.domains.free(domain)
    }

    fn map_pages(
        &mut self,
        domain: DomainId,
        iova: u64,
        frames: &[PhysAddr],
        perm: DmaPermission,
    ) -> Result<(), IommuError> {
        let slpt = self
            .slpts
            .get_mut(&domain.as_u16())
            .ok_or(IommuError::InvalidDomain)?;
        slpt.map_pages(iova, frames, perm)?;

        // SAFETY: mmio_base was mapped during init.
        let regs = unsafe { regs::VtdRegs::new(self.mmio_base) };
        tlb::invalidate_iotlb_domain(regs.base(), self.ecap, domain);
        Ok(())
    }

    fn unmap_pages(
        &mut self,
        domain: DomainId,
        iova: u64,
        page_count: usize,
    ) -> Result<(), IommuError> {
        let slpt = self
            .slpts
            .get_mut(&domain.as_u16())
            .ok_or(IommuError::InvalidDomain)?;
        slpt.unmap_pages(iova, page_count)?;

        // SAFETY: mmio_base was mapped during init.
        let regs = unsafe { regs::VtdRegs::new(self.mmio_base) };
        tlb::invalidate_iotlb_domain(regs.base(), self.ecap, domain);
        Ok(())
    }

    fn attach_device(&mut self, domain: DomainId, bdf: PciBdf) -> Result<(), IommuError> {
        let slpt = self
            .slpts
            .get(&domain.as_u16())
            .ok_or(IommuError::InvalidDomain)?;
        let slpt_phys = slpt.root_phys();
        let agaw = slpt.agaw();

        let bus = bdf.bus as usize;

        // Allocate context table for this bus if not already present.
        if self.context_tables[bus].is_none() {
            let ct_phys = tables::alloc_table_frame();
            self.context_tables[bus] = Some(ct_phys);

            // Write root table entry for this bus.
            let root_virt = hadron_mm::hhdm::phys_to_virt(self.root_table_phys);
            let root_entry =
                // SAFETY: root_virt points to a 4 KiB page with 256 RootEntry slots.
                unsafe { &mut *(root_virt.as_u64() as *mut tables::RootEntry).add(bus) };
            root_entry.set_context_table(ct_phys);
        }

        let ct_phys = self.context_tables[bus].unwrap();
        let devfn = (bdf.device as usize) * 8 + bdf.function as usize;

        // Write context entry for this device/function.
        let ct_virt = hadron_mm::hhdm::phys_to_virt(ct_phys);
        let ctx_entry =
            // SAFETY: ct_virt points to a 4 KiB page with 256 ContextEntry slots.
            unsafe { &mut *(ct_virt.as_u64() as *mut tables::ContextEntry).add(devfn) };
        ctx_entry.set_translation(domain.as_u16(), slpt_phys, agaw);

        // Invalidate context cache for this domain.
        // SAFETY: mmio_base was mapped during init.
        let regs = unsafe { regs::VtdRegs::new(self.mmio_base) };
        tlb::invalidate_context_domain(&regs, domain);
        Ok(())
    }

    fn detach_device(&mut self, bdf: PciBdf) -> Result<(), IommuError> {
        let bus = bdf.bus as usize;
        let ct_phys = self.context_tables[bus].ok_or(IommuError::DeviceNotAttached)?;
        let devfn = (bdf.device as usize) * 8 + bdf.function as usize;

        // Read domain ID before clearing.
        let ct_virt = hadron_mm::hhdm::phys_to_virt(ct_phys);
        let ctx_entry =
            // SAFETY: ct_virt points to a 4 KiB page with 256 ContextEntry slots.
            unsafe { &mut *(ct_virt.as_u64() as *mut tables::ContextEntry).add(devfn) };

        let domain_id = ctx_entry.domain_id().ok_or(IommuError::DeviceNotAttached)?;

        // Clear the context entry.
        *ctx_entry = tables::ContextEntry::EMPTY;

        // Invalidate context cache.
        // SAFETY: mmio_base was mapped during init.
        let regs = unsafe { regs::VtdRegs::new(self.mmio_base) };
        tlb::invalidate_context_domain(&regs, DomainId(domain_id));
        Ok(())
    }
}

/// Select the best supported AGAW from the capability register.
///
/// Prefers 48-bit (4-level) over 39-bit (3-level).
fn select_agaw(cap: u64) -> tables::AddressWidth {
    let sagaw = regs::cap_sagaw(cap);
    if sagaw & 0x04 != 0 {
        tables::AddressWidth::Agaw48
    } else if sagaw & 0x02 != 0 {
        tables::AddressWidth::Agaw39
    } else {
        // Fallback — should not happen (init checks for supported AGAW).
        tables::AddressWidth::Agaw39
    }
}
