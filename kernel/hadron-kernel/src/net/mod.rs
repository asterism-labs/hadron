//! Minimal network stack: ARP resolution/reply and ICMP echo (ping).
//!
//! Provides Ethernet framing, ARP, IPv4, and ICMP with a static IP
//! configuration. The stack runs as a single async background task that
//! takes ownership of the first available NIC.

extern crate alloc;

mod arp;
mod checksum;
mod ethernet;
mod icmp;
mod ipv4;
mod task;

use alloc::string::String;

use crate::drivers::device_registry::DeviceRegistry;

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

/// Initializes the network stack.
///
/// Takes the first available NIC from the device registry, configures a
/// static IP address, and spawns the async RX loop. If no NIC is found,
/// logs a warning and returns gracefully.
pub fn init() {
    // Find and take the first available NIC.
    let (name, nic) = match DeviceRegistry::with_mut(|dr| {
        let name = dr.net_device_names().next().map(String::from);
        name.and_then(|n| {
            let dev = dr.take_net_device(&n)?;
            Some((n, dev))
        })
    }) {
        Some(pair) => pair,
        None => {
            crate::kwarn!("net: no network device found, skipping stack init");
            return;
        }
    };

    let mac = nic.mac_address();
    crate::kinfo!("net: starting stack on {} (MAC={})", name, mac);

    let config = NetConfig {
        ip: [192, 168, 100, 2],
        netmask: [255, 255, 255, 0],
        gateway: [192, 168, 100, 1],
    };

    crate::sched::spawn_background("net-rx", task::net_rx_loop(nic, config));
}
