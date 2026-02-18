//! RSDT / XSDT table enumeration.
//!
//! The Root System Description Table (RSDT, 32-bit entries) and its 64-bit
//! counterpart (XSDT) contain pointers to all other ACPI tables. This module
//! provides an iterator over those entries and a helper to locate a table by
//! its 4-byte signature.

use core::ptr;

use crate::AcpiHandler;
use crate::sdt::SdtHeader;

/// Size in bytes of a single table-pointer entry in the RSDT (32-bit).
const RSDT_ENTRY_SIZE: usize = 4;

/// Size in bytes of a single table-pointer entry in the XSDT (64-bit).
const XSDT_ENTRY_SIZE: usize = 8;

/// Iterator over table entry physical addresses in an RSDT or XSDT.
pub struct RsdtIterator {
    /// Pointer to the first entry (immediately after the SDT header).
    base: *const u8,
    /// Number of entries remaining.
    remaining: usize,
    /// Current offset (in bytes) from `base`.
    offset: usize,
    /// Size of each entry: 4 for RSDT, 8 for XSDT.
    entry_size: usize,
}

impl RsdtIterator {
    /// Create a new iterator over the entries of an RSDT or XSDT.
    ///
    /// # Safety
    ///
    /// `base` must point to the first entry byte of a mapped RSDT/XSDT (i.e.,
    /// the address immediately after the [`SdtHeader`]). The region
    /// `base..base + entry_count * entry_size` must be readable.
    pub(crate) unsafe fn new(base: *const u8, entry_count: usize, is_xsdt: bool) -> Self {
        Self {
            base,
            remaining: entry_count,
            offset: 0,
            entry_size: if is_xsdt {
                XSDT_ENTRY_SIZE
            } else {
                RSDT_ENTRY_SIZE
            },
        }
    }
}

impl Iterator for RsdtIterator {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;

        // SAFETY: the constructor guarantees readable memory for all entries.
        let addr = unsafe {
            let entry_ptr = self.base.add(self.offset);
            if self.entry_size == XSDT_ENTRY_SIZE {
                ptr::read_unaligned(entry_ptr.cast::<u64>())
            } else {
                u64::from(ptr::read_unaligned(entry_ptr.cast::<u32>()))
            }
        };
        self.offset += self.entry_size;
        Some(addr)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for RsdtIterator {}

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
    let header_ptr = unsafe { handler.map_physical_region(rsdt_addr, SdtHeader::SIZE) };
    // SAFETY: header_ptr is valid for SdtHeader::SIZE bytes.
    let header = unsafe { SdtHeader::read_from(header_ptr) };

    let total_len = header.length() as usize;
    let entries_len = total_len.saturating_sub(SdtHeader::SIZE);
    let entry_size = if is_xsdt {
        XSDT_ENTRY_SIZE
    } else {
        RSDT_ENTRY_SIZE
    };

    if entries_len == 0 || entry_size == 0 {
        return None;
    }

    let entry_count = entries_len / entry_size;

    // Map the entire table so we can iterate entries.
    // SAFETY: caller provides a valid physical address, total_len is from the header.
    let table_ptr = unsafe { handler.map_physical_region(rsdt_addr, total_len) };
    // SAFETY: table_ptr + SdtHeader::SIZE is the start of entries, and the
    // region is valid for entry_count * entry_size bytes.
    let entries_ptr = unsafe { table_ptr.add(SdtHeader::SIZE) };

    // SAFETY: entries_ptr is valid and properly sized as shown above.
    let iter = unsafe { RsdtIterator::new(entries_ptr, entry_count, is_xsdt) };

    for entry_phys in iter {
        // Map just the header of the candidate table.
        // SAFETY: entry_phys is a physical address from the RSDT/XSDT.
        let candidate_ptr = unsafe { handler.map_physical_region(entry_phys, SdtHeader::SIZE) };
        // SAFETY: candidate_ptr is valid for SdtHeader::SIZE bytes.
        let candidate = unsafe { SdtHeader::read_from(candidate_ptr) };
        if &candidate.signature() == signature {
            return Some(entry_phys);
        }
    }

    None
}
