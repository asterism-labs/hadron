//! IOMMU kernel objects: Iommu, Bti, and Pmt.
//!
//! These objects mediate DMA access for userspace drivers. A driver receives an
//! `Iommu` handle at startup, creates a `Bti` for its PCI device, and pins
//! memory via `Pmt` tokens for safe DMA transfers.
//!
//! Objects live in `hadron-kernel` (not `hadron-objects`) because they depend on
//! both the object system and the IOMMU hardware layer.

pub mod bti;
pub mod iommu;
pub mod pmt;
