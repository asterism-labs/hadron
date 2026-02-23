//! IPv4 header parsing and building (no options, no fragmentation).

use super::checksum::internet_checksum;

/// IPv4 header length in bytes (no options).
pub const IPV4_HEADER_LEN: usize = 20;

/// IP protocol number for ICMP.
pub const PROTO_ICMP: u8 = 1;

/// Parsed IPv4 header (20 bytes, IHL=5, no options).
pub struct Ipv4Header {
    /// Source IP address.
    pub src: [u8; 4],
    /// Destination IP address.
    pub dst: [u8; 4],
    /// Protocol number (e.g., 1 = ICMP).
    pub protocol: u8,
    /// Total length (header + payload).
    pub total_len: u16,
}

impl Ipv4Header {
    /// Parses an IPv4 header from `buf`.
    ///
    /// Validates version = 4 and IHL = 5 (no options). Returns the header
    /// and a slice over the payload, or `None` on failure.
    pub fn parse(buf: &[u8]) -> Option<(Self, &[u8])> {
        if buf.len() < IPV4_HEADER_LEN {
            return None;
        }

        let version_ihl = buf[0];
        let version = version_ihl >> 4;
        let ihl = version_ihl & 0x0F;

        // We only support version 4 with no options (IHL = 5).
        if version != 4 || ihl != 5 {
            return None;
        }

        let total_len = u16::from_be_bytes([buf[2], buf[3]]);
        let protocol = buf[9];
        let src = <[u8; 4]>::try_from(&buf[12..16]).ok()?;
        let dst = <[u8; 4]>::try_from(&buf[16..20]).ok()?;

        let payload_end = (total_len as usize).min(buf.len());
        if payload_end <= IPV4_HEADER_LEN {
            return None;
        }

        Some((
            Self {
                src,
                dst,
                protocol,
                total_len,
            },
            &buf[IPV4_HEADER_LEN..payload_end],
        ))
    }

    /// Writes an IPv4 header into `buf` at `offset`.
    ///
    /// `payload_len` is the size of the data following the header.
    /// Returns the offset after the header (`offset + 20`), or `None` if
    /// `buf` is too small.
    pub fn write(
        buf: &mut [u8],
        offset: usize,
        src: [u8; 4],
        dst: [u8; 4],
        protocol: u8,
        payload_len: usize,
    ) -> Option<usize> {
        if buf.len() < offset + IPV4_HEADER_LEN {
            return None;
        }

        let total_len = (IPV4_HEADER_LEN + payload_len) as u16;
        let b = &mut buf[offset..offset + IPV4_HEADER_LEN];

        // Version (4) + IHL (5) = 0x45
        b[0] = 0x45;
        // DSCP / ECN
        b[1] = 0;
        // Total length
        b[2..4].copy_from_slice(&total_len.to_be_bytes());
        // Identification
        b[4..6].copy_from_slice(&[0, 0]);
        // Flags (Don't Fragment) + Fragment offset
        b[6..8].copy_from_slice(&[0x40, 0x00]);
        // TTL
        b[8] = 64;
        // Protocol
        b[9] = protocol;
        // Header checksum placeholder
        b[10..12].copy_from_slice(&[0, 0]);
        // Source IP
        b[12..16].copy_from_slice(&src);
        // Destination IP
        b[16..20].copy_from_slice(&dst);

        // Compute and fill header checksum.
        let cksum = internet_checksum(&buf[offset..offset + IPV4_HEADER_LEN]);
        buf[offset + 10..offset + 12].copy_from_slice(&cksum.to_be_bytes());

        Some(offset + IPV4_HEADER_LEN)
    }
}
