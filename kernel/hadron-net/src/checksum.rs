//! RFC 1071 Internet checksum.
//!
//! Used by IPv4 headers and ICMP packets.

/// Computes the RFC 1071 Internet checksum over `data`.
///
/// Returns the ones-complement checksum as a big-endian `u16`.
pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;

    // Sum 16-bit words.
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }

    // Add trailing odd byte.
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }

    // Fold 32-bit accumulator into 16 bits.
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !(sum as u16)
}
