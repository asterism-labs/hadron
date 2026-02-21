//! `hadron-acpi` --- a standalone, `no_std` ACPI table parser.
//!
//! This crate provides types and functions for parsing the core ACPI tables
//! that a kernel needs during early boot: RSDP, RSDT/XSDT, MADT, HPET,
//! FADT, and MCFG. It does **not** depend on `alloc`; all table iteration
//! is done through safe byte-slice iterators backed by an [`AcpiHandler`] that
//! maps physical memory on demand.
//!
//! # Usage
//!
//! ```ignore
//! let tables = AcpiTables::new(rsdp_physical_address, my_handler)?;
//! let madt = tables.madt()?;
//! for entry in madt.entries() {
//!     // ...
//! }
//! ```

#![no_std]
#![warn(missing_docs)]

pub mod fadt;
pub mod hpet;
pub mod madt;
pub mod mcfg;
pub mod rsdp;
pub mod rsdt;
pub mod sdt;

// Re-export key types at crate root for convenience.
pub use fadt::Fadt;
pub use hpet::HpetTable;
pub use madt::{Madt, MadtEntry, MadtEntryIter};
pub use mcfg::{Mcfg, McfgEntry};
pub use sdt::{SdtHeader, ValidatedTable};

/// Errors that can occur during ACPI table parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpiError {
    /// The checksum of a table or the RSDP did not validate (sum != 0).
    InvalidChecksum,
    /// The table signature did not match the expected value.
    InvalidSignature,
    /// The RSDP revision field contained an unrecognised value.
    InvalidRevision,
    /// A table with the requested signature was not found in the RSDT/XSDT.
    TableNotFound,
    /// The RSDP structure was invalid (bad signature or checksum).
    InvalidRsdp,
    /// A table or structure was too short to contain the expected data.
    TruncatedData,
}

/// Trait for mapping physical memory regions so ACPI tables can be read.
///
/// An implementation must return a byte slice covering at least `size` bytes
/// starting at physical address `phys`. The mapping may be an identity map, a
/// higher-half direct map (HHDM), or a temporary mapping --- the crate is
/// agnostic to the strategy.
///
/// # Safety
///
/// Implementors must ensure that the returned slice is valid and readable for
/// the requested `size` bytes. The mapping must remain valid for `'static`.
pub unsafe trait AcpiHandler {
    /// Map a physical memory region and return a byte slice over it.
    ///
    /// # Safety
    ///
    /// The caller guarantees that `phys` is a valid ACPI-related physical
    /// address and `size` does not extend beyond the actual table. The
    /// implementation must return a slice that is valid and readable for
    /// `size` bytes.
    unsafe fn map_physical_region(&self, phys: u64, size: usize) -> &'static [u8];
}

/// Collection of ACPI tables discovered via the RSDP.
///
/// This is the primary entry point for ACPI table access. Construct it with
/// [`AcpiTables::new`] by providing the physical address of the RSDP and an
/// [`AcpiHandler`] implementation, then use the convenience methods to retrieve
/// individual tables.
pub struct AcpiTables<H: AcpiHandler> {
    /// Handler used to map physical memory.
    handler: H,
    /// Physical address of the RSDT or XSDT.
    rsdt_addr: u64,
    /// `true` if `rsdt_addr` points to an XSDT, `false` for RSDT.
    is_xsdt: bool,
}

impl<H: AcpiHandler> AcpiTables<H> {
    /// Discover and validate the ACPI table hierarchy starting from the RSDP.
    ///
    /// This validates the RSDP at `rsdp_phys` and extracts the RSDT or XSDT
    /// address. Individual tables are parsed lazily when requested.
    ///
    /// # Errors
    ///
    /// Returns an [`AcpiError`] if the RSDP is invalid.
    pub fn new(rsdp_phys: u64, handler: H) -> Result<Self, AcpiError> {
        let (rsdt_addr, is_xsdt) = rsdp::parse_rsdp(&handler, rsdp_phys)?;
        Ok(Self {
            handler,
            rsdt_addr,
            is_xsdt,
        })
    }

    /// Search the RSDT/XSDT for a table with the given 4-byte signature.
    ///
    /// Returns the physical address of the table if found, or `None`.
    #[must_use]
    pub fn find_table(&self, signature: &[u8; 4]) -> Option<u64> {
        rsdt::find_table_in_rsdt(&self.handler, self.rsdt_addr, self.is_xsdt, signature)
    }

    /// Parse and return the MADT (Multiple APIC Description Table).
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::TableNotFound`] if no MADT exists, or another
    /// [`AcpiError`] variant if the table is malformed.
    pub fn madt(&self) -> Result<Madt, AcpiError> {
        let phys = self
            .find_table(madt::MADT_SIGNATURE)
            .ok_or(AcpiError::TableNotFound)?;
        Madt::parse(&self.handler, phys)
    }

    /// Parse and return the HPET table.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::TableNotFound`] if no HPET table exists, or
    /// another [`AcpiError`] variant if the table is malformed.
    pub fn hpet(&self) -> Result<HpetTable, AcpiError> {
        let phys = self
            .find_table(hpet::HPET_SIGNATURE)
            .ok_or(AcpiError::TableNotFound)?;
        HpetTable::parse(&self.handler, phys)
    }

    /// Parse and return the FADT (Fixed ACPI Description Table).
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::TableNotFound`] if no FADT exists, or another
    /// [`AcpiError`] variant if the table is malformed.
    pub fn fadt(&self) -> Result<Fadt, AcpiError> {
        let phys = self
            .find_table(fadt::FADT_SIGNATURE)
            .ok_or(AcpiError::TableNotFound)?;
        Fadt::parse(&self.handler, phys)
    }

    /// Parse and return the MCFG (PCI Express ECAM) table.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::TableNotFound`] if no MCFG table exists, or
    /// another [`AcpiError`] variant if the table is malformed.
    pub fn mcfg(&self) -> Result<Mcfg, AcpiError> {
        let phys = self
            .find_table(mcfg::MCFG_SIGNATURE)
            .ok_or(AcpiError::TableNotFound)?;
        Mcfg::parse(&self.handler, phys)
    }

    /// Returns a reference to the underlying [`AcpiHandler`].
    #[must_use]
    pub fn handler(&self) -> &H {
        &self.handler
    }

    /// Returns the physical address of the RSDT or XSDT.
    #[must_use]
    pub fn rsdt_addr(&self) -> u64 {
        self.rsdt_addr
    }

    /// Returns whether the root table is an XSDT (`true`) or RSDT (`false`).
    #[must_use]
    pub fn is_xsdt(&self) -> bool {
        self.is_xsdt
    }
}
