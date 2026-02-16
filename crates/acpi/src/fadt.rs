//! Fixed ACPI Description Table (FADT) parsing.
//!
//! The FADT contains fixed hardware configuration data needed by the ACPI
//! driver. We parse only the subset of fields that the kernel currently needs:
//! the PM timer I/O port, the century CMOS register index, boot architecture
//! flags, and the general ACPI feature flags.

use core::ptr;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// FADT table signature.
pub const FADT_SIGNATURE: &[u8; 4] = b"FACP";

/// Parsed FADT — only the fields we currently need.
#[derive(Debug, Clone, Copy)]
pub struct Fadt {
    /// I/O port address of the PM Timer (Power Management Timer).
    ///
    /// If zero, the PM timer is not available via a fixed I/O port.
    pub pm_timer_block: u32,
    /// CMOS RAM index of the century BCD value (RTC).
    ///
    /// If zero, the century register is not supported.
    pub century: u8,
    /// IA-PC boot architecture flags.
    ///
    /// Bit 0: legacy 8259 IRQ routing, bit 1: 8042 controller, etc.
    pub boot_architecture_flags: u16,
    /// Fixed feature flags.
    ///
    /// See the ACPI specification for individual bit meanings.
    pub flags: u32,
}

impl Fadt {
    /// Byte offset of `pm_tmr_blk` within the FADT (from the start of the table).
    const PM_TMR_BLK_OFFSET: usize = 76;
    /// Byte offset of `century` within the FADT.
    const CENTURY_OFFSET: usize = 108;
    /// Byte offset of `boot_architecture_flags` (`IAPC_BOOT_ARCH`) within the FADT.
    const BOOT_ARCH_OFFSET: usize = 109;
    /// Byte offset of `flags` within the FADT.
    const FLAGS_OFFSET: usize = 112;

    /// Minimum FADT length required to read all the fields we need.
    ///
    /// We need up to offset 112 + 4 bytes = 116 bytes.
    const MIN_LENGTH: usize = 116;

    /// Parse a FADT from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidSignature`] if the table signature is not
    /// `FACP`, or [`AcpiError::InvalidChecksum`] if the checksum is invalid.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        // Map the SDT header first.
        // SAFETY: caller provides a valid physical address.
        let header_ptr = unsafe { handler.map_physical_region(phys, SdtHeader::SIZE) };
        // SAFETY: header_ptr is valid for SdtHeader::SIZE bytes.
        let header = unsafe { SdtHeader::read_from(header_ptr) };

        if &header.signature() != FADT_SIGNATURE {
            return Err(AcpiError::InvalidSignature);
        }

        let total_len = header.length() as usize;

        // Map the full table.
        // SAFETY: phys is valid, total_len comes from the header.
        let table_ptr = unsafe { handler.map_physical_region(phys, total_len) };

        // Validate checksum.
        // SAFETY: table_ptr is valid for total_len bytes.
        if !unsafe { crate::sdt::validate_checksum(table_ptr, total_len) } {
            return Err(AcpiError::InvalidChecksum);
        }

        // Ensure the table is long enough for the fields we need.
        if total_len < Self::MIN_LENGTH {
            // Older FADT revisions may be shorter; provide zero defaults for
            // missing fields rather than failing outright.
            return Ok(Self::parse_partial(table_ptr, total_len));
        }

        // SAFETY: table_ptr is valid for at least MIN_LENGTH bytes.
        unsafe { Ok(Self::read_fields(table_ptr)) }
    }

    /// Read all needed fields from a fully-sized FADT.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid for at least [`Self::MIN_LENGTH`] bytes.
    unsafe fn read_fields(ptr: *const u8) -> Self {
        // SAFETY: caller guarantees sufficient length.
        unsafe {
            Self {
                pm_timer_block: ptr::read_unaligned(ptr.add(Self::PM_TMR_BLK_OFFSET).cast()),
                century: ptr.add(Self::CENTURY_OFFSET).read(),
                boot_architecture_flags: ptr::read_unaligned(
                    ptr.add(Self::BOOT_ARCH_OFFSET).cast(),
                ),
                flags: ptr::read_unaligned(ptr.add(Self::FLAGS_OFFSET).cast()),
            }
        }
    }

    /// Parse a shorter-than-expected FADT, filling in zero for missing fields.
    fn parse_partial(ptr: *const u8, len: usize) -> Self {
        // SAFETY: we only read fields whose offsets are within `len`.
        unsafe {
            Self {
                pm_timer_block: if len >= Self::PM_TMR_BLK_OFFSET + 4 {
                    ptr::read_unaligned(ptr.add(Self::PM_TMR_BLK_OFFSET).cast())
                } else {
                    0
                },
                century: if len > Self::CENTURY_OFFSET {
                    ptr.add(Self::CENTURY_OFFSET).read()
                } else {
                    0
                },
                boot_architecture_flags: if len >= Self::BOOT_ARCH_OFFSET + 2 {
                    ptr::read_unaligned(ptr.add(Self::BOOT_ARCH_OFFSET).cast())
                } else {
                    0
                },
                flags: if len >= Self::FLAGS_OFFSET + 4 {
                    ptr::read_unaligned(ptr.add(Self::FLAGS_OFFSET).cast())
                } else {
                    0
                },
            }
        }
    }
}
