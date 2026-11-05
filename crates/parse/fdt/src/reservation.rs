//! Memory reservation block parsing.

use zerocopy::big_endian::U64;
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

/// A single memory reservation entry from the FDT reservation block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemReservation {
    /// Physical start address of the reserved region.
    pub address: u64,
    /// Size in bytes of the reserved region.
    pub size: u64,
}

/// Raw reservation entry as stored in the DTB (two big-endian 64-bit values).
#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RawReservationEntry {
    address: U64,
    size: U64,
}

/// Iterator over memory reservation entries in the FDT.
///
/// Yields [`MemReservation`] entries until the all-zero terminator is reached.
pub struct MemReservationIter<'a> {
    data: &'a [u8],
}

impl<'a> MemReservationIter<'a> {
    /// Creates a new iterator over the raw reservation block bytes.
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }
}

impl Iterator for MemReservationIter<'_> {
    type Item = MemReservation;

    fn next(&mut self) -> Option<Self::Item> {
        let (entry, rest) = RawReservationEntry::read_from_prefix(self.data).ok()?;
        let address = entry.address.get();
        let size = entry.size.get();

        // All-zero entry terminates the list.
        if address == 0 && size == 0 {
            return None;
        }

        self.data = rest;
        Some(MemReservation { address, size })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_block_without_terminator_ends_iteration() {
        let mut data = [0u8; 16 + 9];
        data[..8].copy_from_slice(&0x8000_0000u64.to_be_bytes());
        data[8..16].copy_from_slice(&0x1000u64.to_be_bytes());
        data[16] = 0xFF;

        let mut iter = MemReservationIter::new(&data);
        assert_eq!(
            iter.next(),
            Some(MemReservation {
                address: 0x8000_0000,
                size: 0x1000,
            })
        );
        assert_eq!(iter.next(), None);
    }
}
