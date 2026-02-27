//! Network subsystem: ethernet stack and AF_UNIX domain sockets.
//!
//! - [`unix`] — AF_UNIX domain socket implementation
//! - The Ethernet/IP stack uses the `hadron-net` crate for protocol logic.

extern crate alloc;

pub mod unix;

use alloc::string::String;

use crate::drivers::device_registry::DeviceRegistry;

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

    let config = hadron_net::NetConfig {
        ip: [192, 168, 100, 2],
        netmask: [255, 255, 255, 0],
        gateway: [192, 168, 100, 1],
    };

    crate::sched::spawn_background("net-rx", hadron_net::task::net_rx_loop(nic, config));
}
