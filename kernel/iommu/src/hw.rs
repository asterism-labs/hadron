//! Generic IOMMU hardware trait.
//!
//! Defines the interface that all IOMMU backends (VT-d, AMD-Vi, etc.) implement.
//! Only Intel VT-d is implemented for now.

use hadron_core::addr::PhysAddr;

use crate::domain::DomainId;

/// Permission flags for DMA mappings.
#[derive(Clone, Copy, Debug)]
pub struct DmaPermission {
    /// Allow DMA reads from this mapping.
    pub read: bool,
    /// Allow DMA writes to this mapping.
    pub write: bool,
}

impl DmaPermission {
    /// Read-only DMA access.
    pub const READ: Self = Self {
        read: true,
        write: false,
    };

    /// Read-write DMA access.
    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
    };
}

/// PCI Bus/Device/Function address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PciBdf {
    /// PCI bus number.
    pub bus: u8,
    /// PCI device number (0-31).
    pub device: u8,
    /// PCI function number (0-7).
    pub function: u8,
}

/// Errors returned by IOMMU operations.
#[derive(Clone, Copy, Debug)]
pub enum IommuError {
    /// No more domain IDs available.
    DomainExhausted,
    /// The specified domain ID is invalid or not allocated.
    InvalidDomain,
    /// The IOVA range is invalid or overlaps existing mappings.
    InvalidIova,
    /// Physical frame allocation failed.
    OutOfMemory,
    /// The device is not attached to any domain.
    DeviceNotAttached,
    /// Hardware reported a fault.
    HardwareFault,
    /// The IOMMU unit is not initialized.
    NotInitialized,
}

/// Abstract IOMMU hardware interface.
///
/// Each backend (VT-d, AMD-Vi) implements this trait to provide DMA isolation.
pub trait IommuHardware: Send + Sync {
    /// Allocate a new DMA domain, returning its domain ID.
    fn alloc_domain(&self) -> Result<DomainId, IommuError>;

    /// Free a DMA domain and all its mappings.
    fn free_domain(&self, domain: DomainId) -> Result<(), IommuError>;

    /// Map an IOVA range to physical frames in a domain's second-level page table.
    fn map_pages(
        &self,
        domain: DomainId,
        iova: u64,
        frames: &[PhysAddr],
        perm: DmaPermission,
    ) -> Result<(), IommuError>;

    /// Unmap an IOVA range from a domain.
    fn unmap_pages(&self, domain: DomainId, iova: u64, page_count: usize)
    -> Result<(), IommuError>;

    /// Assign a PCI device (BDF) to a domain.
    fn attach_device(&self, domain: DomainId, bdf: PciBdf) -> Result<(), IommuError>;

    /// Detach a PCI device from its domain.
    fn detach_device(&self, bdf: PciBdf) -> Result<(), IommuError>;
}
