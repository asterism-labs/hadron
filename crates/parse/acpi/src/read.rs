//! Little-endian field reads from ACPI table bytes.
//!
//! ACPI tables are little-endian and often place multi-byte fields at
//! unaligned offsets. These helpers read through zerocopy's byte-order types
//! and return `None` when the field runs past the end of the slice.

use zerocopy::FromBytes;
use zerocopy::little_endian::{U16, U32, U64};

/// Reads a `T` from `data` at byte offset `offset`.
pub(crate) fn at<T: FromBytes>(data: &[u8], offset: usize) -> Option<T> {
    T::read_from_prefix(data.get(offset..)?)
        .ok()
        .map(|(value, _)| value)
}

/// Reads a `u8` at `offset`.
pub(crate) fn u8_at(data: &[u8], offset: usize) -> Option<u8> {
    data.get(offset).copied()
}

/// Reads a little-endian `u16` at `offset`.
pub(crate) fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    at::<U16>(data, offset).map(U16::get)
}

/// Reads a little-endian `u32` at `offset`.
pub(crate) fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    at::<U32>(data, offset).map(U32::get)
}

/// Reads a little-endian `u64` at `offset`.
pub(crate) fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    at::<U64>(data, offset).map(U64::get)
}

/// Splits the next entry with a one-byte type and one-byte length off the
/// front of `data`.
///
/// Returns the whole entry, header included. Returns `None` and leaves `data`
/// unchanged when the header is truncated, the length is below 2, or the
/// length runs past the end of `data`.
pub(crate) fn split_u8_entry<'a>(data: &mut &'a [u8]) -> Option<&'a [u8]> {
    let length = usize::from(*data.get(1)?);
    if length < 2 {
        return None;
    }
    let (entry, rest) = data.split_at_checked(length)?;
    *data = rest;
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_at_unaligned_offset() {
        let data = [0xFF, 0x78, 0x56, 0x34, 0x12, 0xAA];
        assert_eq!(u32_at(&data, 1), Some(0x1234_5678));
        assert_eq!(u16_at(&data, 4), Some(0xAA12));
    }

    #[test]
    fn short_reads_return_none() {
        let data = [0u8; 4];
        assert_eq!(u32_at(&data, 1), None);
        assert_eq!(u64_at(&data, 0), None);
        assert_eq!(u8_at(&data, 4), None);
        assert_eq!(u16_at(&data, usize::MAX), None);
    }

    #[test]
    fn split_u8_entry_walks_and_stops_on_bad_length() {
        let mut data: &[u8] = &[7, 3, 0xAB, 9, 2, 5, 0];
        assert_eq!(split_u8_entry(&mut data), Some(&[7, 3, 0xAB][..]));
        assert_eq!(split_u8_entry(&mut data), Some(&[9, 2][..]));
        // Length 0 is malformed: no progress, no panic.
        assert_eq!(split_u8_entry(&mut data), None);
        assert_eq!(data, &[5, 0]);
        // Length past the end of the buffer.
        let mut data: &[u8] = &[1, 9, 0];
        assert_eq!(split_u8_entry(&mut data), None);
    }
}
