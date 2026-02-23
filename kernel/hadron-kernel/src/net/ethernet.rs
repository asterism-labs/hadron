//! Ethernet frame parsing and building.

/// Length of an Ethernet header in bytes.
pub const ETHERNET_HEADER_LEN: usize = 14;

/// EtherType: ARP (0x0806).
pub const ETHERTYPE_ARP: [u8; 2] = [0x08, 0x06];

/// EtherType: IPv4 (0x0800).
pub const ETHERTYPE_IPV4: [u8; 2] = [0x08, 0x00];

/// Broadcast MAC address (ff:ff:ff:ff:ff:ff).
pub const BROADCAST_MAC: [u8; 6] = [0xFF; 6];

/// Parsed Ethernet header (14 bytes: dst[6] + src[6] + ethertype[2]).
pub struct EthernetHeader {
    /// Destination MAC address.
    pub dst: [u8; 6],
    /// Source MAC address.
    pub src: [u8; 6],
    /// EtherType (big-endian).
    pub ethertype: [u8; 2],
}

impl EthernetHeader {
    /// Parses an Ethernet header from `buf`.
    ///
    /// Returns the header and a slice over the payload, or `None` if the
    /// buffer is too short.
    pub fn parse(buf: &[u8]) -> Option<(Self, &[u8])> {
        if buf.len() < ETHERNET_HEADER_LEN {
            return None;
        }

        let dst = <[u8; 6]>::try_from(&buf[0..6]).ok()?;
        let src = <[u8; 6]>::try_from(&buf[6..12]).ok()?;
        let ethertype = <[u8; 2]>::try_from(&buf[12..14]).ok()?;

        Some((
            Self {
                dst,
                src,
                ethertype,
            },
            &buf[ETHERNET_HEADER_LEN..],
        ))
    }

    /// Writes an Ethernet header into `buf` and returns the offset after the
    /// header (14).
    ///
    /// Returns `None` if `buf` is too small.
    pub fn write(buf: &mut [u8], dst: [u8; 6], src: [u8; 6], ethertype: [u8; 2]) -> Option<usize> {
        if buf.len() < ETHERNET_HEADER_LEN {
            return None;
        }

        buf[0..6].copy_from_slice(&dst);
        buf[6..12].copy_from_slice(&src);
        buf[12..14].copy_from_slice(&ethertype);

        Some(ETHERNET_HEADER_LEN)
    }
}
