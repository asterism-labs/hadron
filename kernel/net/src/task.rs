//! Async RX loop for the network stack.
//!
//! Owns the NIC and runs a `recv → parse → dispatch → send` loop. Yields
//! naturally via `.await` when no packets are available — zero CPU when idle.

extern crate alloc;

use alloc::boxed::Box;

use hadron_driver_api::dyn_dispatch::DynNetDevice;
use hadron_driver_api::net::NetworkDevice;

use super::NetConfig;
use super::arp::ArpTable;
use super::ethernet::{ETHERTYPE_ARP, ETHERTYPE_IPV4, EthernetHeader};
use super::icmp;
use super::ipv4::{Ipv4Header, PROTO_ICMP};

/// Maximum Ethernet frame size (MTU 1500 + 14 byte header + 4 byte FCS margin).
const MAX_FRAME: usize = 1518;

/// Async receive loop that processes incoming Ethernet frames.
///
/// Takes ownership of the NIC and loops forever, dispatching received
/// frames to the appropriate protocol handler (ARP, ICMP). Yields via
/// `.await` when no packets are available.
pub async fn net_rx_loop(nic: Box<dyn DynNetDevice>, config: NetConfig) {
    let our_mac = nic.mac_address().0;
    let our_ip = config.ip;

    let mut rx_buf = [0u8; MAX_FRAME];
    let mut tx_buf = [0u8; MAX_FRAME];
    let mut arp_table = ArpTable::new();

    loop {
        // Wait for an incoming frame.
        let len = match nic.recv(&mut rx_buf).await {
            Ok(n) => n,
            Err(_) => continue,
        };

        let frame = &rx_buf[..len];

        // Parse Ethernet header.
        let (eth, payload) = match EthernetHeader::parse(frame) {
            Some(parsed) => parsed,
            None => continue,
        };

        match eth.ethertype {
            ETHERTYPE_ARP => {
                if let Some(reply_len) = arp_table.handle_arp(our_ip, our_mac, payload, &mut tx_buf)
                {
                    let _ = nic.send(&tx_buf[..reply_len]).await;
                }
            }
            ETHERTYPE_IPV4 => {
                let (ip_hdr, ip_payload) = match Ipv4Header::parse(payload) {
                    Some(parsed) => parsed,
                    None => continue,
                };

                // Only process packets destined for us.
                if ip_hdr.dst != our_ip {
                    continue;
                }

                if ip_hdr.protocol == PROTO_ICMP {
                    if let Some(reply_len) = icmp::handle_icmp(
                        eth.src,
                        ip_hdr.src,
                        our_mac,
                        our_ip,
                        ip_payload,
                        &mut tx_buf,
                    ) {
                        let _ = nic.send(&tx_buf[..reply_len]).await;
                    }
                }
            }
            _ => {
                // Unknown EtherType — silently drop.
            }
        }
    }
}
