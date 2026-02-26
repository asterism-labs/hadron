//! Minimal network stack: ARP resolution/reply and ICMP echo (ping).
//!
//! Provides Ethernet framing, ARP, IPv4, and ICMP with a static IP
//! configuration. The stack runs as a single async background task that
//! takes ownership of a NIC via [`task::net_rx_loop`].
//!
//! This crate contains pure protocol logic with no kernel dependencies.
//! The kernel glue (device registry lookup, task spawning) lives in
//! `hadron-kernel`.

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

extern crate alloc;

pub mod arp;
pub mod checksum;
pub mod ethernet;
pub mod icmp;
pub mod ipv4;
pub mod task;

/// Static network configuration.
#[derive(Clone, Copy)]
pub struct NetConfig {
    /// Our IPv4 address.
    pub ip: [u8; 4],
    /// Subnet mask.
    pub netmask: [u8; 4],
    /// Default gateway.
    pub gateway: [u8; 4],
}

#[cfg(test)]
mod tests {
    use super::arp::ArpTable;
    use super::checksum::internet_checksum;
    use super::ethernet::{ETHERNET_HEADER_LEN, ETHERTYPE_ARP, ETHERTYPE_IPV4, EthernetHeader};
    use super::icmp::handle_icmp;
    use super::ipv4::{IPV4_HEADER_LEN, Ipv4Header, PROTO_ICMP};

    // -- Checksum tests -------------------------------------------------------

    #[test]
    fn checksum_zeros() {
        // All-zero data should produce 0xFFFF (ones-complement of zero).
        assert_eq!(internet_checksum(&[0; 20]), 0xFFFF);
    }

    #[test]
    fn checksum_rfc1071_example() {
        // RFC 1071 example: 0x0001 + 0xF203 + 0xF4F5 + 0xF6F7.
        let data = [0x00, 0x01, 0xF2, 0x03, 0xF4, 0xF5, 0xF6, 0xF7];
        let cksum = internet_checksum(&data);
        // Sum = 0x0001 + 0xF203 + 0xF4F5 + 0xF6F7 = 0x2DDF0
        // Fold: 0xDDF0 + 0x0002 = 0xDDF2 -> ~0xDDF2 = 0x220D
        assert_eq!(cksum, 0x220D);
    }

    #[test]
    fn checksum_odd_length() {
        let data = [0x00, 0x01, 0x02];
        // 0x0001 + 0x0200 = 0x0201 -> ~0x0201 = 0xFDFE
        assert_eq!(internet_checksum(&data), 0xFDFE);
    }

    // -- Ethernet tests -------------------------------------------------------

    #[test]
    fn ethernet_parse_valid() {
        let mut frame = [0u8; 20];
        frame[0..6].copy_from_slice(&[0xAA; 6]); // dst
        frame[6..12].copy_from_slice(&[0xBB; 6]); // src
        frame[12..14].copy_from_slice(&ETHERTYPE_IPV4); // ethertype
        frame[14..20].copy_from_slice(&[1, 2, 3, 4, 5, 6]); // payload

        let (hdr, payload) = EthernetHeader::parse(&frame).unwrap();
        assert_eq!(hdr.dst, [0xAA; 6]);
        assert_eq!(hdr.src, [0xBB; 6]);
        assert_eq!(hdr.ethertype, ETHERTYPE_IPV4);
        assert_eq!(payload, &[1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn ethernet_parse_too_short() {
        assert!(EthernetHeader::parse(&[0u8; 13]).is_none());
    }

    #[test]
    fn ethernet_write_roundtrip() {
        let mut buf = [0u8; 20];
        let off = EthernetHeader::write(&mut buf, [0xAA; 6], [0xBB; 6], ETHERTYPE_ARP).unwrap();
        assert_eq!(off, ETHERNET_HEADER_LEN);

        let (hdr, _) = EthernetHeader::parse(&buf).unwrap();
        assert_eq!(hdr.dst, [0xAA; 6]);
        assert_eq!(hdr.src, [0xBB; 6]);
        assert_eq!(hdr.ethertype, ETHERTYPE_ARP);
    }

    // -- IPv4 tests -----------------------------------------------------------

    #[test]
    fn ipv4_parse_valid() {
        let mut buf = [0u8; 30];
        // Version=4, IHL=5
        buf[0] = 0x45;
        // Total length = 30
        buf[2..4].copy_from_slice(&30u16.to_be_bytes());
        // Protocol = ICMP
        buf[9] = PROTO_ICMP;
        // Src IP
        buf[12..16].copy_from_slice(&[10, 0, 0, 1]);
        // Dst IP
        buf[16..20].copy_from_slice(&[10, 0, 0, 2]);
        // Payload
        buf[20..30].copy_from_slice(&[0xDE; 10]);

        let (hdr, payload) = Ipv4Header::parse(&buf).unwrap();
        assert_eq!(hdr.src, [10, 0, 0, 1]);
        assert_eq!(hdr.dst, [10, 0, 0, 2]);
        assert_eq!(hdr.protocol, PROTO_ICMP);
        assert_eq!(hdr.total_len, 30);
        assert_eq!(payload.len(), 10);
    }

    #[test]
    fn ipv4_parse_too_short() {
        assert!(Ipv4Header::parse(&[0u8; 19]).is_none());
    }

    #[test]
    fn ipv4_parse_wrong_version() {
        let mut buf = [0u8; 30];
        buf[0] = 0x65; // version 6
        buf[2..4].copy_from_slice(&30u16.to_be_bytes());
        assert!(Ipv4Header::parse(&buf).is_none());
    }

    #[test]
    fn ipv4_write_valid_checksum() {
        let mut buf = [0u8; 60];
        let off =
            Ipv4Header::write(&mut buf, 0, [10, 0, 0, 1], [10, 0, 0, 2], PROTO_ICMP, 10).unwrap();
        assert_eq!(off, IPV4_HEADER_LEN);

        // Verify checksum: computing checksum over the header (including the
        // stored checksum) should yield 0.
        let cksum = internet_checksum(&buf[..IPV4_HEADER_LEN]);
        assert_eq!(cksum, 0, "IPv4 header checksum verification failed");
    }

    // -- ARP tests ------------------------------------------------------------

    #[test]
    fn arp_handle_request_generates_reply() {
        let our_ip = [192, 168, 1, 10];
        let our_mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
        let sender_mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x02];
        let sender_ip = [192, 168, 1, 20];

        // Build an ARP Request targeting our_ip.
        let mut arp_req = [0u8; 28];
        arp_req[0..2].copy_from_slice(&1u16.to_be_bytes()); // htype = Ethernet
        arp_req[2..4].copy_from_slice(&0x0800u16.to_be_bytes()); // ptype = IPv4
        arp_req[4] = 6; // hlen
        arp_req[5] = 4; // plen
        arp_req[6..8].copy_from_slice(&1u16.to_be_bytes()); // oper = Request
        arp_req[8..14].copy_from_slice(&sender_mac); // sha
        arp_req[14..18].copy_from_slice(&sender_ip); // spa
        arp_req[18..24].copy_from_slice(&[0; 6]); // tha (unknown)
        arp_req[24..28].copy_from_slice(&our_ip); // tpa

        let mut reply = [0u8; 128];
        let mut table = ArpTable::new();
        let len = table
            .handle_arp(our_ip, our_mac, &arp_req, &mut reply)
            .expect("expected ARP reply");

        // Reply = 14 (Ethernet) + 28 (ARP) = 42 bytes.
        assert_eq!(len, 42);

        // Verify Ethernet header: dst=sender_mac, src=our_mac, ethertype=ARP.
        let (eth, _) = EthernetHeader::parse(&reply[..len]).unwrap();
        assert_eq!(eth.dst, sender_mac);
        assert_eq!(eth.src, our_mac);
        assert_eq!(eth.ethertype, ETHERTYPE_ARP);

        // Verify ARP reply operation field.
        let oper = u16::from_be_bytes([reply[20], reply[21]]);
        assert_eq!(oper, 2, "expected ARP Reply (oper=2)");
    }

    #[test]
    fn arp_ignores_non_request() {
        let our_ip = [10, 0, 0, 1];
        let our_mac = [0x02; 6];

        // ARP Reply (oper=2) — should be learned but not replied to.
        let mut arp_reply = [0u8; 28];
        arp_reply[0..2].copy_from_slice(&1u16.to_be_bytes());
        arp_reply[2..4].copy_from_slice(&0x0800u16.to_be_bytes());
        arp_reply[4] = 6;
        arp_reply[5] = 4;
        arp_reply[6..8].copy_from_slice(&2u16.to_be_bytes()); // Reply
        arp_reply[8..14].copy_from_slice(&[0x03; 6]);
        arp_reply[14..18].copy_from_slice(&[10, 0, 0, 2]);
        arp_reply[24..28].copy_from_slice(&our_ip);

        let mut reply = [0u8; 128];
        let mut table = ArpTable::new();
        assert!(
            table
                .handle_arp(our_ip, our_mac, &arp_reply, &mut reply)
                .is_none()
        );
    }

    // -- ICMP tests -----------------------------------------------------------

    #[test]
    fn icmp_echo_request_generates_reply() {
        let eth_src = [0x02, 0x00, 0x00, 0x00, 0x00, 0x02];
        let ip_src = [10, 0, 0, 1];
        let our_mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
        let our_ip = [10, 0, 0, 2];

        // Build ICMP Echo Request: type=8, code=0, checksum=0, id=1, seq=1, data="ping"
        let mut icmp_req = [0u8; 12];
        icmp_req[0] = 8; // Echo Request
        icmp_req[1] = 0; // Code
        // checksum placeholder
        icmp_req[4..6].copy_from_slice(&1u16.to_be_bytes()); // id
        icmp_req[6..8].copy_from_slice(&1u16.to_be_bytes()); // seq
        icmp_req[8..12].copy_from_slice(b"ping"); // data

        let mut reply = [0u8; 256];
        let len = handle_icmp(eth_src, ip_src, our_mac, our_ip, &icmp_req, &mut reply)
            .expect("expected ICMP reply");

        // Reply = 14 (Eth) + 20 (IPv4) + 12 (ICMP) = 46 bytes.
        assert_eq!(len, 46);

        // Parse Ethernet header.
        let (eth, eth_payload) = EthernetHeader::parse(&reply[..len]).unwrap();
        assert_eq!(eth.dst, eth_src);
        assert_eq!(eth.src, our_mac);
        assert_eq!(eth.ethertype, ETHERTYPE_IPV4);

        // Parse IPv4 header.
        let (ip, ip_payload) = Ipv4Header::parse(eth_payload).unwrap();
        assert_eq!(ip.src, our_ip);
        assert_eq!(ip.dst, ip_src);
        assert_eq!(ip.protocol, PROTO_ICMP);

        // Verify ICMP reply: type=0 (Echo Reply), data preserved.
        assert_eq!(ip_payload[0], 0, "expected ICMP Echo Reply type=0");
        assert_eq!(&ip_payload[8..12], b"ping");

        // Verify ICMP checksum.
        let cksum = internet_checksum(ip_payload);
        assert_eq!(cksum, 0, "ICMP checksum verification failed");
    }

    #[test]
    fn icmp_ignores_non_echo_request() {
        // ICMP type=3 (Destination Unreachable) — should be ignored.
        let mut icmp_data = [0u8; 8];
        icmp_data[0] = 3;

        let mut reply = [0u8; 256];
        assert!(handle_icmp([0; 6], [0; 4], [0; 6], [0; 4], &icmp_data, &mut reply).is_none());
    }

    #[test]
    fn icmp_rejects_short_payload() {
        let mut reply = [0u8; 256];
        assert!(
            handle_icmp(
                [0; 6],
                [0; 4],
                [0; 6],
                [0; 4],
                &[8, 0, 0], // too short (< 8 bytes)
                &mut reply
            )
            .is_none()
        );
    }
}
