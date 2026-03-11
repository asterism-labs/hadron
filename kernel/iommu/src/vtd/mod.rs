//! Intel VT-d (DMA Remapping) driver.
//!
//! Implements the [`IommuHardware`](crate::hw::IommuHardware) trait for Intel
//! VT-d remapping hardware. Each DRHD entry from the ACPI DMAR table corresponds
//! to one [`VtdUnit`], which owns the MMIO register mapping, root/context tables,
//! and a domain ID allocator.

pub mod fault;
pub mod init;
pub mod regs;
pub mod tables;
pub mod tlb;

use alloc::vec::Vec;

use hadron_core::addr::VirtAddr;

use crate::domain::DomainAllocator;

/// A single VT-d remapping hardware unit.
///
/// Each unit corresponds to one DRHD entry in the DMAR table and manages
/// its own register set, root table, and domain allocator.
#[allow(dead_code)] // Phase 4b: fields used by IommuHardware trait impl
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
    pub(crate) root_table_phys: hadron_core::addr::PhysAddr,
    /// Domain ID allocator for this unit.
    pub(crate) domains: DomainAllocator,
    /// Context tables allocated for this unit (indexed by bus number).
    pub(crate) context_tables: Vec<Option<hadron_core::addr::PhysAddr>>,
}
