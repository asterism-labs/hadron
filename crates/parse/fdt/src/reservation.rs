//! Memory reservation block parsing.

use crate::header::Be64;
use hadron_binparse::FromBytes;

/// A single memory reservation entry from the FDT reservation block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemReservation {
    /// Physical start address of the reserved region.
    pub address: u64,
    /// Size in bytes of the reserved region.
    pub size: u64,
}

/// Raw reservation entry as stored in the DTB (two big-endian 64-bit values).
#[derive(Clone, Copy, Debug, FromBytes)]
#[repr(C)]
struct RawReservationEntry {
    address: Be64,
    size: Be64,
}

/// Iterator over memory reservation entries in the FDT.
///
/// Yields [`MemReservation`] entries until the all-zero terminator is reached.
pub struct MemReservationIter<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> MemReservationIter<'a> {
    /// Creates a new iterator over the raw reservation block bytes.
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }
}

impl Iterator for MemReservationIter<'_> {
    type Item = MemReservation;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = RawReservationEntry::read_at(self.data, self.offset)?;
        let address = entry.address.get();
        let size = entry.size.get();

        // All-zero entry terminates the list.
        if address == 0 && size == 0 {
            return None;
        }

        self.offset += core::mem::size_of::<RawReservationEntry>();
        Some(MemReservation { address, size })
    }
}
