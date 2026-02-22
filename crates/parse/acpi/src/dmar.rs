//! DMA Remapping Table (DMAR) parsing for Intel VT-d.
//!
//! The DMAR table describes the DMA remapping hardware units (DRHDs) and
//! reserved memory regions (RMRRs) used by Intel's VT-d IOMMU. Entry headers
//! use `u16` type and `u16` length fields, so we use a hand-written iterator
//! rather than the `TableEntries` macro.

use hadron_binparse::FromBytes;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// DMAR table signature.
pub const DMAR_SIGNATURE: &[u8; 4] = b"DMAR";

/// Parsed DMAR table.
pub struct Dmar {
    /// Width of the host address (N means addresses are N+1 bits wide).
    pub host_address_width: u8,
    /// DMAR flags (bit 0: INTR_REMAP, bit 1: X2APIC_OPT_OUT).
    pub flags: u8,
    /// Byte slice covering the remapping structure entries.
    entries_data: &'static [u8],
}

impl Dmar {
    /// Size of the fixed DMAR fields after the SDT header.
    ///
    /// `host_address_width` (1) + `flags` (1) + reserved (10) = 12 bytes.
    const FIELDS_SIZE: usize = 12;

    /// Parse a DMAR from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidSignature`] if the table signature is not
    /// `DMAR`, or [`AcpiError::InvalidChecksum`] if the checksum is invalid.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        let table = crate::sdt::load_table(handler, phys, DMAR_SIGNATURE)?;

        let haw_offset = SdtHeader::SIZE;
        let host_address_width =
            u8::read_at(table.data, haw_offset).ok_or(AcpiError::TruncatedData)?;
        let flags = u8::read_at(table.data, haw_offset + 1).ok_or(AcpiError::TruncatedData)?;

        let entries_offset = SdtHeader::SIZE + Self::FIELDS_SIZE;
        let entries_data = table.data.get(entries_offset..).unwrap_or(&[]);

        Ok(Self {
            host_address_width,
            flags,
            entries_data,
        })
    }

    /// Returns an iterator over the DMAR remapping structure entries.
    #[must_use]
    pub fn entries(&self) -> DmarEntryIter {
        DmarEntryIter {
            data: self.entries_data,
            offset: 0,
        }
    }
}

/// A single DMAR remapping structure entry.
#[derive(Debug, Clone, Copy)]
pub enum DmarEntry<'a> {
    /// Type 0: DMA Remapping Hardware Unit Definition (DRHD).
    Drhd {
        /// Flags (bit 0: INCLUDE_PCI_ALL).
        flags: u8,
        /// PCI segment number.
        segment: u16,
        /// Base address of the remapping hardware register set.
        register_base_address: u64,
        /// Device scope data for iterating sub-entries.
        device_scope_data: &'a [u8],
    },
    /// Type 1: Reserved Memory Region Reporting (RMRR).
    Rmrr {
        /// PCI segment number.
        segment: u16,
        /// Base address of the reserved memory region.
        base_address: u64,
        /// Limit address of the reserved memory region (inclusive).
        limit_address: u64,
        /// Device scope data for iterating sub-entries.
        device_scope_data: &'a [u8],
    },
    /// Type 2: Root Port ATS Capability Reporting (ATSR).
    Atsr {
        /// Flags (bit 0: ALL_PORTS).
        flags: u8,
        /// PCI segment number.
        segment: u16,
        /// Device scope data for iterating sub-entries.
        device_scope_data: &'a [u8],
    },
    /// An entry type that we do not parse.
    Unknown {
        /// The entry type (u16).
        entry_type: u16,
        /// The entry length including the 4-byte header.
        length: u16,
    },
}

/// Iterator over DMAR remapping structure entries.
pub struct DmarEntryIter<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for DmarEntryIter<'a> {
    type Item = DmarEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // Need at least 4 bytes for type (u16) + length (u16).
        if self.offset + 4 > self.data.len() {
            return None;
        }

        let entry_type = u16::read_at(self.data, self.offset)?;
        let length = u16::read_at(self.data, self.offset + 2)?;
        let length_usize = length as usize;

        if length_usize < 4 || self.offset + length_usize > self.data.len() {
            return None;
        }

        let entry_data = &self.data[self.offset..self.offset + length_usize];
        self.offset += length_usize;

        Some(match entry_type {
            // DRHD: flags(u8) at +4, reserved(u8) at +5, segment(u16) at +6,
            // register_base_address(u64) at +8, device scopes at +16
            0 => {
                let flags = u8::read_at(entry_data, 4).unwrap_or(0);
                let segment = u16::read_at(entry_data, 6).unwrap_or(0);
                let register_base_address = u64::read_at(entry_data, 8).unwrap_or(0);
                let device_scope_data = entry_data.get(16..).unwrap_or(&[]);
                DmarEntry::Drhd {
                    flags,
                    segment,
                    register_base_address,
                    device_scope_data,
                }
            }
            // RMRR: reserved(u16) at +4, segment(u16) at +6,
            // base_address(u64) at +8, limit_address(u64) at +16, device scopes at +24
            1 => {
                let segment = u16::read_at(entry_data, 6).unwrap_or(0);
                let base_address = u64::read_at(entry_data, 8).unwrap_or(0);
                let limit_address = u64::read_at(entry_data, 16).unwrap_or(0);
                let device_scope_data = entry_data.get(24..).unwrap_or(&[]);
                DmarEntry::Rmrr {
                    segment,
                    base_address,
                    limit_address,
                    device_scope_data,
                }
            }
            // ATSR: flags(u8) at +4, segment(u16) at +6, device scopes at +8
            2 => {
                let flags = u8::read_at(entry_data, 4).unwrap_or(0);
                let segment = u16::read_at(entry_data, 6).unwrap_or(0);
                let device_scope_data = entry_data.get(8..).unwrap_or(&[]);
                DmarEntry::Atsr {
                    flags,
                    segment,
                    device_scope_data,
                }
            }
            _ => DmarEntry::Unknown { entry_type, length },
        })
    }
}

/// A single device scope entry within a DRHD, RMRR, or ATSR.
#[derive(Debug, Clone, Copy)]
pub struct DeviceScope {
    /// Device scope type (1=PCI Endpoint, 2=PCI Sub-hierarchy, 3=IOAPIC,
    /// 4=MSI Capable HPET, 5=ACPI Namespace Device).
    pub scope_type: u8,
    /// Enumeration ID (for I/O APIC, HPET, or ACPI device scopes).
    pub enumeration_id: u8,
    /// PCI bus number where the device path starts.
    pub start_bus: u8,
    /// PCI path as (device, function) pairs. Up to 4 entries for bridge chains.
    pub path: [(u8, u8); 4],
    /// Number of valid path entries.
    pub path_len: u8,
}

/// Iterator over device scope entries within a DMAR entry.
pub struct DeviceScopeIter<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> DeviceScopeIter<'a> {
    /// Create a new device scope iterator from the device scope data slice.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }
}

impl Iterator for DeviceScopeIter<'_> {
    type Item = DeviceScope;

    fn next(&mut self) -> Option<Self::Item> {
        // Minimum device scope entry: type(1) + length(1) + reserved(2) +
        // enumeration_id(1) + start_bus(1) + path(2) = 8 bytes.
        if self.offset + 6 > self.data.len() {
            return None;
        }

        let scope_type = u8::read_at(self.data, self.offset)?;
        let length = u8::read_at(self.data, self.offset + 1)? as usize;

        if length < 6 || self.offset + length > self.data.len() {
            return None;
        }

        let enumeration_id = u8::read_at(self.data, self.offset + 4).unwrap_or(0);
        let start_bus = u8::read_at(self.data, self.offset + 5).unwrap_or(0);

        // Path entries start at offset 6, each is 2 bytes (device, function).
        let path_bytes = length.saturating_sub(6);
        let path_count = (path_bytes / 2).min(4);
        let mut path = [(0u8, 0u8); 4];
        for i in 0..path_count {
            let off = self.offset + 6 + i * 2;
            let dev = u8::read_at(self.data, off).unwrap_or(0);
            let func = u8::read_at(self.data, off + 1).unwrap_or(0);
            path[i] = (dev, func);
        }

        self.offset += length;

        Some(DeviceScope {
            scope_type,
            enumeration_id,
            start_bus,
            path,
            path_len: path_count as u8,
        })
    }
}
