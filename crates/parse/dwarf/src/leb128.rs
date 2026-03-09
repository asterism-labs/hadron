//! LEB128 (Little-Endian Base 128) encoding used by DWARF.
//!
//! DWARF uses LEB128 for compact variable-length integer encoding.
//! Unsigned LEB128 (ULEB128) and signed LEB128 (SLEB128) are both supported.

/// Decodes an unsigned LEB128 value from the given byte slice.
///
/// Returns `(value, bytes_consumed)` on success, or `None` if the encoding
/// is truncated (no byte with the high bit clear before the end of data)
/// or would overflow a `u64`.
#[must_use]
pub fn decode_uleb128(data: &[u8]) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;

    for (i, &byte) in data.iter().enumerate() {
        if shift >= 64 {
            return None; // overflow
        }
        let value = u64::from(byte & 0x7f);
        // Check for overflow before shifting
        if shift > 0 && value > (u64::MAX >> shift) {
            return None;
        }
        result |= value << shift;
        shift += 7;

        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
    }
    None // truncated
}

/// Decodes a signed LEB128 value from the given byte slice.
///
/// Returns `(value, bytes_consumed)` on success, or `None` if the encoding
/// is truncated or would overflow an `i64`.
#[must_use]
pub fn decode_sleb128(data: &[u8]) -> Option<(i64, usize)> {
    let mut result: i64 = 0;
    let mut shift: u32 = 0;

    for (i, &byte) in data.iter().enumerate() {
        if shift >= 64 {
            return None; // overflow
        }
        let value = i64::from(byte & 0x7f);
        result |= value << shift;
        shift += 7;

        if byte & 0x80 == 0 {
            // Sign-extend if the sign bit of the last byte is set
            if shift < 64 && (byte & 0x40) != 0 {
                result |= !0i64 << shift;
            }
            return Some((result, i + 1));
        }
    }
    None // truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uleb128_zero() {
        assert_eq!(decode_uleb128(&[0x00]), Some((0, 1)));
    }

    #[test]
    fn uleb128_one_byte() {
        assert_eq!(decode_uleb128(&[0x01]), Some((1, 1)));
        assert_eq!(decode_uleb128(&[0x7f]), Some((127, 1)));
    }

    #[test]
    fn uleb128_two_bytes() {
        // 128 = 0x80 0x01
        assert_eq!(decode_uleb128(&[0x80, 0x01]), Some((128, 2)));
        // 624485 = 0xE5 0x8E 0x26
        assert_eq!(decode_uleb128(&[0xE5, 0x8E, 0x26]), Some((624485, 3)));
    }

    #[test]
    fn uleb128_truncated() {
        assert_eq!(decode_uleb128(&[0x80]), None);
        assert_eq!(decode_uleb128(&[]), None);
    }

    #[test]
    fn sleb128_zero() {
        assert_eq!(decode_sleb128(&[0x00]), Some((0, 1)));
    }

    #[test]
    fn sleb128_positive() {
        assert_eq!(decode_sleb128(&[0x01]), Some((1, 1)));
        assert_eq!(decode_sleb128(&[0x3f]), Some((63, 1)));
    }

    #[test]
    fn sleb128_negative() {
        // -1 = 0x7f
        assert_eq!(decode_sleb128(&[0x7f]), Some((-1, 1)));
        // -2 = 0x7e
        assert_eq!(decode_sleb128(&[0x7e]), Some((-2, 1)));
        // -123456 = 0xC0 0xBB 0x78
        assert_eq!(decode_sleb128(&[0xC0, 0xBB, 0x78]), Some((-123456, 3)));
    }

    #[test]
    fn sleb128_truncated() {
        assert_eq!(decode_sleb128(&[0x80]), None);
        assert_eq!(decode_sleb128(&[]), None);
    }

    #[test]
    fn uleb128_multi_byte_values() {
        // Test value 0x80 (128)
        assert_eq!(decode_uleb128(&[0x80, 0x01]), Some((128, 2)));
        // Test value 0x100 (256) = 0x80 0x02
        assert_eq!(decode_uleb128(&[0x80, 0x02]), Some((256, 2)));
        // Test value 0x3FFF (16383) = 0xFF 0x7F
        assert_eq!(decode_uleb128(&[0xFF, 0x7F]), Some((16383, 2)));
    }

    #[test]
    fn sleb128_sign_extension() {
        // -64 = 0x40
        assert_eq!(decode_sleb128(&[0x40]), Some((-64, 1)));
        // 64 = 0xC0 0x00
        assert_eq!(decode_sleb128(&[0xC0, 0x00]), Some((64, 2)));
    }
}
