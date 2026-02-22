//! RSDT / XSDT table enumeration.
//!
//! The Root System Description Table (RSDT, 32-bit entries) and its 64-bit
//! counterpart (XSDT) contain pointers to all other ACPI tables. This module
//! provides an iterator over those entries and a helper to locate a table by
//! its 4-byte signature.

use hadron_binparse::FromBytes;

use crate::AcpiHandler;
use crate::sdt::SdtHeader;

/// Size in bytes of a single table-pointer entry in the RSDT (32-bit).
const RSDT_ENTRY_SIZE: usize = 4;

/// Size in bytes of a single table-pointer entry in the XSDT (64-bit).
const XSDT_ENTRY_SIZE: usize = 8;

/// Iterator over table entry physical addresses in an RSDT or XSDT.
pub struct RsdtIterator<'a> {
    /// Byte slice covering all entries.
    data: &'a [u8],
    /// Current offset (in bytes) from the start of `data`.
    offset: usize,
    /// Size of each entry: 4 for RSDT, 8 for XSDT.
    entry_size: usize,
}

impl<'a> RsdtIterator<'a> {
    /// Create a new iterator over the entries of an RSDT or XSDT.
    pub(crate) fn new(data: &'a [u8], is_xsdt: bool) -> Self {
        Self {
            data,
            offset: 0,
            entry_size: if is_xsdt {
                XSDT_ENTRY_SIZE
            } else {
                RSDT_ENTRY_SIZE
            },
        }
    }
}

impl Iterator for RsdtIterator<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset + self.entry_size > self.data.len() {
            return None;
        }

        let entry_data = &self.data[self.offset..];
        let addr = if self.entry_size == XSDT_ENTRY_SIZE {
            u64::read_from(entry_data)?
        } else {
            u64::from(u32::read_from(entry_data)?)
        };
        self.offset += self.entry_size;
        Some(addr)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.data.len() - self.offset) / self.entry_size;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for RsdtIterator<'_> {}

/// Search the RSDT/XSDT for a table whose SDT header matches `signature`.
///
/// Returns the physical address of the matching table, or `None` if no table
/// with that signature exists.
///
/// # Arguments
///
/// * `handler` — trait object used to map physical memory regions.
/// * `rsdt_addr` — physical address of the RSDT or XSDT.
/// * `is_xsdt` — whether the table is an XSDT (`true`) or RSDT (`false`).
/// * `signature` — the 4-byte ASCII table signature to search for.
pub fn find_table_in_rsdt(
    handler: &impl AcpiHandler,
    rsdt_addr: u64,
    is_xsdt: bool,
    signature: &[u8; 4],
) -> Option<u64> {
    // Map the RSDT/XSDT header to learn the total table length.
    // SAFETY: caller provides a valid physical address.
    let header_data = unsafe { handler.map_physical_region(rsdt_addr, SdtHeader::SIZE) };
    let header = SdtHeader::read_from_bytes(header_data)?;

    let total_len = header.length() as usize;
    let entries_len = total_len.saturating_sub(SdtHeader::SIZE);

    if entries_len == 0 {
        return None;
    }

    // Map the entire table so we can iterate entries.
    // SAFETY: caller provides a valid physical address, total_len is from the header.
    let table_data = unsafe { handler.map_physical_region(rsdt_addr, total_len) };
    let entries_data = table_data.get(SdtHeader::SIZE..)?;

    let iter = RsdtIterator::new(entries_data, is_xsdt);

    for entry_phys in iter {
        // Map just the header of the candidate table.
        // SAFETY: entry_phys is a physical address from the RSDT/XSDT.
        let candidate_data = unsafe { handler.map_physical_region(entry_phys, SdtHeader::SIZE) };
        let candidate = SdtHeader::read_from_bytes(candidate_data)?;
        if &candidate.signature() == signature {
            return Some(entry_phys);
        }
    }

    None
}

/// Search the RSDT/XSDT for all tables whose SDT header matches `signature`.
///
/// Returns a [`MatchingTableIter`] that yields the physical address of every
/// matching table. This is useful for discovering multiple SSDTs, which all
/// share the signature `b"SSDT"`.
pub fn find_all_tables_in_rsdt<'a, H: AcpiHandler>(
    handler: &'a H,
    rsdt_addr: u64,
    is_xsdt: bool,
    signature: &'a [u8; 4],
) -> MatchingTableIter<'a, H> {
    // Map the RSDT/XSDT header to learn the total table length.
    // SAFETY: caller provides a valid physical address.
    let header_data = unsafe { handler.map_physical_region(rsdt_addr, SdtHeader::SIZE) };
    let total_len = SdtHeader::read_from_bytes(header_data)
        .map(|h| h.length() as usize)
        .unwrap_or(SdtHeader::SIZE);

    let entries_data = if total_len > SdtHeader::SIZE {
        // SAFETY: caller provides a valid physical address, total_len from header.
        let table_data = unsafe { handler.map_physical_region(rsdt_addr, total_len) };
        table_data.get(SdtHeader::SIZE..).unwrap_or(&[])
    } else {
        &[]
    };

    MatchingTableIter {
        handler,
        iter: RsdtIterator::new(entries_data, is_xsdt),
        signature,
    }
}

/// Iterator that yields physical addresses of all tables matching a given
/// signature in the RSDT/XSDT.
pub struct MatchingTableIter<'a, H: AcpiHandler> {
    handler: &'a H,
    iter: RsdtIterator<'a>,
    signature: &'a [u8; 4],
}

impl<H: AcpiHandler> Iterator for MatchingTableIter<'_, H> {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        for entry_phys in self.iter.by_ref() {
            // SAFETY: entry_phys is a physical address from the RSDT/XSDT.
            let candidate_data = unsafe {
                self.handler
                    .map_physical_region(entry_phys, SdtHeader::SIZE)
            };
            if let Some(candidate) = SdtHeader::read_from_bytes(candidate_data) {
                if &candidate.signature() == self.signature {
                    return Some(entry_phys);
                }
            }
        }
        None
    }
}
