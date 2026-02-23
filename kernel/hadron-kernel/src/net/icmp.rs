//! ICMP echo request → reply handling.

use super::checksum::internet_checksum;
use super::ethernet::{self, ETHERNET_HEADER_LEN, ETHERTYPE_IPV4};
use super::ipv4::{self, IPV4_HEADER_LEN, PROTO_ICMP};

/// ICMP type: Echo Request.
const ICMP_ECHO_REQUEST: u8 = 8;
/// ICMP type: Echo Reply.
const ICMP_ECHO_REPLY: u8 = 0;
/// Minimum ICMP header length (type + code + checksum + id + seq).
const ICMP_HEADER_LEN: usize = 8;

/// Handles an incoming ICMP packet and builds a reply frame.
///
/// `eth_src` / `ip_src` are the sender's addresses (from the received frame).
/// `icmp_payload` is the full ICMP data (type + code + checksum + rest).
///
/// If the packet is an Echo Request, builds a complete Ethernet + IPv4 + ICMP
/// Echo Reply frame in `reply_buf` and returns the total frame length.
/// Otherwise returns `None`.
pub fn handle_icmp(
    eth_src: [u8; 6],
    ip_src: [u8; 4],
    our_mac: [u8; 6],
    our_ip: [u8; 4],
    icmp_payload: &[u8],
    reply_buf: &mut [u8],
) -> Option<usize> {
    if icmp_payload.len() < ICMP_HEADER_LEN {
        return None;
    }

    let icmp_type = icmp_payload[0];
    if icmp_type != ICMP_ECHO_REQUEST {
        return None;
    }

    let icmp_data = &icmp_payload[ICMP_HEADER_LEN..];
    let icmp_reply_len = ICMP_HEADER_LEN + icmp_data.len();
    let total_len = ETHERNET_HEADER_LEN + IPV4_HEADER_LEN + icmp_reply_len;

    if reply_buf.len() < total_len {
        return None;
    }

    // 1. Ethernet header.
    let off = ethernet::EthernetHeader::write(reply_buf, eth_src, our_mac, ETHERTYPE_IPV4)?;

    // 2. IPv4 header.
    let off = ipv4::Ipv4Header::write(reply_buf, off, our_ip, ip_src, PROTO_ICMP, icmp_reply_len)?;

    // 3. ICMP Echo Reply.
    let icmp_start = off;
    // Type = 0 (Echo Reply)
    reply_buf[off] = ICMP_ECHO_REPLY;
    // Code = 0
    reply_buf[off + 1] = 0;
    // Checksum placeholder
    reply_buf[off + 2] = 0;
    reply_buf[off + 3] = 0;
    // Copy identifier + sequence number + data from request.
    reply_buf[off + 4..off + ICMP_HEADER_LEN].copy_from_slice(&icmp_payload[4..ICMP_HEADER_LEN]);
    reply_buf[off + ICMP_HEADER_LEN..off + icmp_reply_len].copy_from_slice(icmp_data);

    // Compute ICMP checksum over the entire ICMP message.
    let cksum = internet_checksum(&reply_buf[icmp_start..icmp_start + icmp_reply_len]);
    reply_buf[icmp_start + 2..icmp_start + 4].copy_from_slice(&cksum.to_be_bytes());

    Some(total_len)
}
