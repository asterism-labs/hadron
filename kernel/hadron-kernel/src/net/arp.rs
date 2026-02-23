//! ARP (Address Resolution Protocol) table and packet handling.
//!
//! Maintains a fixed-size ARP cache and handles ARP request/reply for
//! Ethernet + IPv4 (hardware type 1, protocol type 0x0800).

use super::ethernet::{self, ETHERTYPE_ARP};

/// ARP packet length for Ethernet + IPv4 (28 bytes).
const ARP_PACKET_LEN: usize = 28;

/// ARP operation: Request.
const ARP_OP_REQUEST: u16 = 1;
/// ARP operation: Reply.
const ARP_OP_REPLY: u16 = 2;

/// Maximum number of entries in the ARP table.
const ARP_TABLE_SIZE: usize = 16;

/// A parsed ARP packet (Ethernet + IPv4 only).
struct ArpPacket {
    /// Hardware type (expected: 1 = Ethernet).
    htype: u16,
    /// Protocol type (expected: 0x0800 = IPv4).
    ptype: u16,
    /// Hardware address length (expected: 6).
    hlen: u8,
    /// Protocol address length (expected: 4).
    plen: u8,
    /// Operation (1 = request, 2 = reply).
    oper: u16,
    /// Sender hardware address (MAC).
    sha: [u8; 6],
    /// Sender protocol address (IPv4).
    spa: [u8; 4],
    /// Target hardware address (MAC).
    tha: [u8; 6],
    /// Target protocol address (IPv4).
    tpa: [u8; 4],
}

impl ArpPacket {
    /// Parses an ARP packet from `data` (the Ethernet payload).
    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < ARP_PACKET_LEN {
            return None;
        }

        let htype = u16::from_be_bytes([data[0], data[1]]);
        let ptype = u16::from_be_bytes([data[2], data[3]]);
        let hlen = data[4];
        let plen = data[5];
        let oper = u16::from_be_bytes([data[6], data[7]]);

        // Only handle Ethernet (htype=1, hlen=6) + IPv4 (ptype=0x0800, plen=4).
        if htype != 1 || ptype != 0x0800 || hlen != 6 || plen != 4 {
            return None;
        }

        let sha = <[u8; 6]>::try_from(&data[8..14]).ok()?;
        let spa = <[u8; 4]>::try_from(&data[14..18]).ok()?;
        let tha = <[u8; 6]>::try_from(&data[18..24]).ok()?;
        let tpa = <[u8; 4]>::try_from(&data[24..28]).ok()?;

        Some(Self {
            htype,
            ptype,
            hlen,
            plen,
            oper,
            sha,
            spa,
            tha,
            tpa,
        })
    }

    /// Writes an ARP reply packet into `buf` starting at `offset`.
    ///
    /// Returns the number of bytes written (always 28).
    fn write_reply(buf: &mut [u8], offset: usize, our_mac: [u8; 6], our_ip: [u8; 4], target_mac: [u8; 6], target_ip: [u8; 4]) -> Option<usize> {
        if buf.len() < offset + ARP_PACKET_LEN {
            return None;
        }

        let b = &mut buf[offset..];
        // Hardware type: Ethernet (1)
        b[0..2].copy_from_slice(&1u16.to_be_bytes());
        // Protocol type: IPv4 (0x0800)
        b[2..4].copy_from_slice(&0x0800u16.to_be_bytes());
        // Hardware address length: 6
        b[4] = 6;
        // Protocol address length: 4
        b[5] = 4;
        // Operation: Reply (2)
        b[6..8].copy_from_slice(&ARP_OP_REPLY.to_be_bytes());
        // Sender hardware address
        b[8..14].copy_from_slice(&our_mac);
        // Sender protocol address
        b[14..18].copy_from_slice(&our_ip);
        // Target hardware address
        b[18..24].copy_from_slice(&target_mac);
        // Target protocol address
        b[24..28].copy_from_slice(&target_ip);

        Some(ARP_PACKET_LEN)
    }
}

/// An entry in the ARP cache.
#[derive(Clone, Copy)]
struct ArpEntry {
    ip: [u8; 4],
    mac: [u8; 6],
}

/// Fixed-size ARP table with circular eviction.
pub struct ArpTable {
    entries: [Option<ArpEntry>; ARP_TABLE_SIZE],
    next: usize,
}

impl ArpTable {
    /// Creates an empty ARP table.
    pub const fn new() -> Self {
        Self {
            entries: [None; ARP_TABLE_SIZE],
            next: 0,
        }
    }

    /// Learns a MAC↔IP mapping (update existing or insert new).
    fn learn(&mut self, ip: [u8; 4], mac: [u8; 6]) {
        // Update existing entry if present.
        for entry in self.entries.iter_mut().flatten() {
            if entry.ip == ip {
                entry.mac = mac;
                return;
            }
        }

        // Insert into next slot (circular eviction).
        self.entries[self.next] = Some(ArpEntry { ip, mac });
        self.next = (self.next + 1) % ARP_TABLE_SIZE;
    }

    /// Handles an incoming ARP packet.
    ///
    /// Learns the sender's MAC→IP mapping. If it is an ARP Request targeting
    /// `our_ip`, builds a complete Ethernet + ARP Reply frame in `reply_buf`
    /// and returns the total frame length. Otherwise returns `None`.
    pub fn handle_arp(
        &mut self,
        our_ip: [u8; 4],
        our_mac: [u8; 6],
        payload: &[u8],
        reply_buf: &mut [u8],
    ) -> Option<usize> {
        let pkt = ArpPacket::parse(payload)?;

        // Always learn the sender.
        self.learn(pkt.spa, pkt.sha);

        // Only reply to ARP Requests targeting our IP.
        if pkt.oper != ARP_OP_REQUEST || pkt.tpa != our_ip {
            return None;
        }

        // Build Ethernet header: unicast reply back to sender.
        let eth_off = ethernet::EthernetHeader::write(reply_buf, pkt.sha, our_mac, ETHERTYPE_ARP)?;

        // Build ARP Reply payload.
        let arp_len = ArpPacket::write_reply(reply_buf, eth_off, our_mac, our_ip, pkt.sha, pkt.spa)?;

        Some(eth_off + arp_len)
    }
}
