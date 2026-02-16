//! Root System Description Pointer (RSDP) parsing and validation.
//!
//! The RSDP is the entry point into the ACPI table hierarchy. ACPI 1.0
//! defines a 20-byte structure (`Rsdp`), while ACPI 2.0+ extends it to
//! 36 bytes (`Rsdp2`) with an XSDT address.

use core::ptr;

use crate::{AcpiError, AcpiHandler};

/// ACPI 1.0 RSDP — 20 bytes.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct Rsdp {
    /// Must be `b"RSD PTR "` (8 bytes, note the trailing space).
    pub signature: [u8; 8],
    /// Checksum covering the first 20 bytes.
    pub checksum: u8,
    /// OEM identification string.
    pub oem_id: [u8; 6],
    /// ACPI revision: 0 for ACPI 1.0, 2 for ACPI 2.0+.
    pub revision: u8,
    /// Physical address of the RSDT (32-bit).
    pub rsdt_address: u32,
}

impl Rsdp {
    /// Size of the ACPI 1.0 RSDP structure in bytes.
    pub const SIZE: usize = 20;

    /// Expected signature bytes.
    pub const SIGNATURE: &[u8; 8] = b"RSD PTR ";
}

/// ACPI 2.0+ RSDP extension — 36 bytes total.
///
/// The first 20 bytes are identical to [`Rsdp`]. The remaining 16 bytes
/// provide the 64-bit XSDT address and an extended checksum.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct Rsdp2 {
    /// The ACPI 1.0 portion.
    pub v1: Rsdp,
    /// Total length of this structure (should be 36).
    pub length: u32,
    /// Physical address of the XSDT (64-bit).
    pub xsdt_address: u64,
    /// Checksum covering the entire 36 bytes.
    pub extended_checksum: u8,
    /// Reserved bytes.
    pub reserved: [u8; 3],
}

impl Rsdp2 {
    /// Size of the ACPI 2.0 RSDP structure in bytes.
    pub const SIZE: usize = 36;
}

/// Parse and validate the RSDP at the given physical address.
///
/// Returns `(table_address, is_xsdt)`:
/// - On ACPI 1.0: the 32-bit RSDT address and `false`.
/// - On ACPI 2.0+: the 64-bit XSDT address and `true`.
///
/// # Errors
///
/// Returns [`AcpiError::InvalidRsdp`] when the signature or checksum is
/// incorrect, or [`AcpiError::InvalidRevision`] for unrecognised revisions.
pub fn parse_rsdp(handler: &impl AcpiHandler, phys: u64) -> Result<(u64, bool), AcpiError> {
    // Map enough memory for the larger v2 structure. We always need at least
    // 20 bytes, and at most 36. Map 36 to cover both cases.
    // SAFETY: we trust the handler to return a valid mapping.
    let ptr = unsafe { handler.map_physical_region(phys, Rsdp2::SIZE) };

    // SAFETY: ptr is valid for at least Rsdp::SIZE bytes.
    let v1: Rsdp = unsafe { ptr::read_unaligned(ptr.cast::<Rsdp>()) };

    // Validate signature.
    if &v1.signature != Rsdp::SIGNATURE {
        return Err(AcpiError::InvalidRsdp);
    }

    // Validate v1 checksum (first 20 bytes).
    // SAFETY: ptr is valid for 36 bytes, so 20 is fine.
    if !unsafe { crate::sdt::validate_checksum(ptr, Rsdp::SIZE) } {
        return Err(AcpiError::InvalidChecksum);
    }

    match v1.revision {
        // ACPI 1.0 — use the 32-bit RSDT address.
        0 => Ok((u64::from(v1.rsdt_address), false)),

        // ACPI 2.0+ — validate extended checksum and use the XSDT address.
        2 => {
            // SAFETY: ptr is valid for Rsdp2::SIZE bytes.
            if !unsafe { crate::sdt::validate_checksum(ptr, Rsdp2::SIZE) } {
                return Err(AcpiError::InvalidChecksum);
            }

            // SAFETY: ptr is valid and properly sized.
            let v2: Rsdp2 = unsafe { ptr::read_unaligned(ptr.cast::<Rsdp2>()) };
            Ok((v2.xsdt_address, true))
        }

        _ => Err(AcpiError::InvalidRevision),
    }
}
