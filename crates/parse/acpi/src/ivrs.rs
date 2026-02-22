//! I/O Virtualization Reporting Structure (IVRS) parsing for AMD-Vi.
//!
//! The IVRS table describes AMD's IOMMU hardware units (IVHDs) and memory
//! definitions (IVMDs). Like DMAR, entry headers use `u16` type and `u16`
//! length fields, so we use a hand-written iterator.

use hadron_binparse::FromBytes;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// IVRS table signature.
pub const IVRS_SIGNATURE: &[u8; 4] = b"IVRS";

/// Parsed IVRS table.
pub struct Ivrs {
    /// I/O virtualization information field.
    pub iv_info: u32,
    /// Byte slice covering the IVHD/IVMD entries.
    entries_data: &'static [u8],
}

impl Ivrs {
    /// Size of the fixed IVRS fields after the SDT header.
    ///
    /// `iv_info` (4) + reserved (8) = 12 bytes.
    const FIELDS_SIZE: usize = 12;

    /// Parse an IVRS from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidSignature`] if the table signature is not
    /// `IVRS`, or [`AcpiError::InvalidChecksum`] if the checksum is invalid.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        let table = crate::sdt::load_table(handler, phys, IVRS_SIGNATURE)?;

        let iv_info = u32::read_at(table.data, SdtHeader::SIZE).ok_or(AcpiError::TruncatedData)?;

        let entries_offset = SdtHeader::SIZE + Self::FIELDS_SIZE;
        let entries_data = table.data.get(entries_offset..).unwrap_or(&[]);

        Ok(Self {
            iv_info,
            entries_data,
        })
    }

    /// Returns an iterator over the IVRS entries (IVHDs and IVMDs).
    #[must_use]
    pub fn entries(&self) -> IvrsEntryIter {
        IvrsEntryIter {
            data: self.entries_data,
            offset: 0,
        }
    }
}

/// A single IVRS entry.
#[derive(Debug, Clone, Copy)]
pub enum IvrsEntry {
    /// IVHD (I/O Virtualization Hardware Definition).
    ///
    /// Types 0x10 (basic), 0x11 (extended), and 0x40 (ACPI HID).
    Ivhd {
        /// IVHD type (0x10, 0x11, or 0x40).
        ivhd_type: u8,
        /// Flags.
        flags: u8,
        /// Device ID of the IOMMU.
        device_id: u16,
        /// Offset into the capability block in PCI config space.
        capability_offset: u16,
        /// Physical base address of the IOMMU registers.
        iommu_base_address: u64,
        /// PCI segment group.
        segment_group: u16,
        /// IOMMU info field.
        iommu_info: u16,
    },
    /// IVMD (I/O Virtualization Memory Definition).
    ///
    /// Types 0x20 (all peripherals), 0x21 (specified peripheral), 0x22 (range).
    Ivmd {
        /// IVMD type (0x20, 0x21, or 0x22).
        ivmd_type: u8,
        /// Flags.
        flags: u8,
        /// Device ID.
        device_id: u16,
        /// Auxiliary data (device-specific).
        auxiliary_data: u16,
        /// Start address of the memory block.
        start_address: u64,
        /// Length of the memory block in bytes.
        memory_block_length: u64,
    },
    /// An entry type that we do not parse.
    Unknown {
        /// The entry type byte.
        entry_type: u8,
        /// The entry length including the header.
        length: u16,
    },
}

/// Iterator over IVRS entries.
pub struct IvrsEntryIter<'a> {
    data: &'a [u8],
    offset: usize,
}

impl Iterator for IvrsEntryIter<'_> {
    type Item = IvrsEntry;

    fn next(&mut self) -> Option<Self::Item> {
        // Need at least 4 bytes for type (u8) + flags (u8) + length (u16).
        if self.offset + 4 > self.data.len() {
            return None;
        }

        let entry_type = u8::read_at(self.data, self.offset)?;
        let flags = u8::read_at(self.data, self.offset + 1).unwrap_or(0);
        let length = u16::read_at(self.data, self.offset + 2)?;
        let length_usize = length as usize;

        if length_usize < 4 || self.offset + length_usize > self.data.len() {
            return None;
        }

        let entry_data = &self.data[self.offset..self.offset + length_usize];
        self.offset += length_usize;

        Some(match entry_type {
            // IVHD types: 0x10, 0x11, 0x40
            // device_id(u16) at +4, capability_offset(u16) at +6,
            // iommu_base_address(u64) at +8, segment_group(u16) at +16,
            // iommu_info(u16) at +18
            0x10 | 0x11 | 0x40 => IvrsEntry::Ivhd {
                ivhd_type: entry_type,
                flags,
                device_id: u16::read_at(entry_data, 4).unwrap_or(0),
                capability_offset: u16::read_at(entry_data, 6).unwrap_or(0),
                iommu_base_address: u64::read_at(entry_data, 8).unwrap_or(0),
                segment_group: u16::read_at(entry_data, 16).unwrap_or(0),
                iommu_info: u16::read_at(entry_data, 18).unwrap_or(0),
            },
            // IVMD types: 0x20, 0x21, 0x22
            // device_id(u16) at +4, auxiliary_data(u16) at +6,
            // reserved(u64) at +8, start_address(u64) at +16,
            // memory_block_length(u64) at +24
            0x20 | 0x21 | 0x22 => IvrsEntry::Ivmd {
                ivmd_type: entry_type,
                flags,
                device_id: u16::read_at(entry_data, 4).unwrap_or(0),
                auxiliary_data: u16::read_at(entry_data, 6).unwrap_or(0),
                start_address: u64::read_at(entry_data, 16).unwrap_or(0),
                memory_block_length: u64::read_at(entry_data, 24).unwrap_or(0),
            },
            _ => IvrsEntry::Unknown { entry_type, length },
        })
    }
}
