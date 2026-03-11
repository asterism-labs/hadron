//! IOMMU hardware abstraction and Intel VT-d driver.
//!
//! This crate provides:
//! - [`IommuHardware`] — generic trait for IOMMU backends
//! - [`vtd`] — Intel VT-d (DMA Remapping) implementation
//! - [`DomainAllocator`] — bitmap-based DMA domain ID allocator
//!
//! Initialization is driven by [`init_vtd()`], which accepts parsed DMAR info
//! and brings up each VT-d remapping unit.

#![no_std]
#![warn(missing_docs)]

extern crate alloc;

pub mod domain;
pub mod hw;
pub mod vtd;

use alloc::vec::Vec;

use hadron_core::sync::IrqSpinLock;

/// Global list of initialized VT-d units.
static VTD_UNITS: IrqSpinLock<Vec<vtd::VtdUnit>> =
    IrqSpinLock::leveled("VTD_UNITS", 14, Vec::new());

/// Parsed DRHD (DMA Remapping Hardware Unit Definition) info passed from ACPI.
#[derive(Clone, Copy, Debug)]
pub struct DrhdEntry {
    /// DRHD flags (bit 0 = INCLUDE_PCI_ALL).
    pub flags: u8,
    /// PCI segment number.
    pub segment: u16,
    /// Physical base address of the VT-d register block.
    pub register_base_address: u64,
}

/// Initialize VT-d IOMMU hardware from parsed DMAR table info.
///
/// `host_address_width` is the DMA addressable width minus one (from DMAR header).
/// `drhds` contains the parsed DRHD entries from the ACPI DMAR table.
///
/// Must be called after PMM, VMM, and heap are initialized.
pub fn init_vtd(host_address_width: u8, drhds: &[DrhdEntry]) {
    vtd::init::init_all_units(host_address_width, drhds);
}

/// Returns the number of initialized VT-d units.
#[must_use]
pub fn unit_count() -> usize {
    VTD_UNITS.lock().len()
}

/// Execute a closure with exclusive access to the VT-d unit at `index`.
///
/// Returns `None` if `index` is out of bounds.
pub fn with_unit<R>(index: usize, f: impl FnOnce(&mut vtd::VtdUnit) -> R) -> Option<R> {
    let mut units = VTD_UNITS.lock();
    units.get_mut(index).map(f)
}
