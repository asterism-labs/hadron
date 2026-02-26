# Network Stack - Phase 1 (ARP & ICMP)

**Status: Completed** (implemented in commit 6fec9b4)

Hadron implements a minimal network stack supporting IPv4 address resolution (ARP) and ICMP echo (ping). The implementation is hand-written, zero-copy, and integrated with the kernel's async executor for non-blocking packet reception.

Source: [`kernel/kernel/src/net/`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/net/), specifically:
- [`net/mod.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/net/mod.rs) -- Network config
- [`net/ipv4.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/net/ipv4.rs) -- IPv4 parsing
- [`net/arp.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/net/arp.rs) -- ARP table and resolution
- [`net/icmp.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/net/icmp.rs) -- ICMP echo request/reply
- [`net/task.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/net/task.rs) -- Async RX loop

## Architecture

The network stack operates in three layers:

```
┌─────────────────────────────────────┐
│ ICMP (Echo Request/Reply)           │
├─────────────────────────────────────┤
│ IPv4 (Header Parsing, Dispatch)     │
├─────────────────────────────────────┤
│ Ethernet (Frame Reception/Dispatch) │
├─────────────────────────────────────┤
│ NIC Driver (e.g., VirtIO, e1000e)   │
└─────────────────────────────────────┘
```

### Key Components

| Component | Role |
|-----------|------|
| `NetConfig` | Static network configuration (IPv4 addr, netmask, gateway) |
| `ArpTable` | 16-entry circular buffer for MAC address resolution |
| `IcmpHandler` | ICMP echo request processing |
| `NetworkRxTask` | Critical-priority async task that polls the NIC for packets |

### Network Configuration

The kernel is configured with a static IPv4 address:

- **IP Address**: `192.168.100.2`
- **Netmask**: `255.255.255.0` (`/24`)
- **Gateway**: `192.168.100.1`

This allows the kernel to participate in ARP and respond to ICMP echo requests from the same subnet.

## ARP (Address Resolution Protocol)

ARP resolves IPv4 addresses to MAC addresses on the local network.

### ARP Table

A circular buffer of 16 ARP entries caches resolved MAC addresses:

```rust
pub struct ArpEntry {
    pub ip: [u8; 4],
    pub mac: [u8; 6],
    pub expires: u64,  // Kernel tick timestamp
}
```

### ARP Resolution

When the kernel needs to send a packet to an IP address:

1. Check the ARP table for a cached entry.
2. If found and not expired, use the cached MAC address.
3. If not found or expired, send an ARP request asking "Who has this IP?"
4. Wait for ARP replies (replies update the table).
5. After cache miss, respond with an ARP reply if the query is for our IP.

### Known Limitations

- **No ARP gratuitous requests** -- The kernel does not announce its MAC address on startup.
- **No ARP timeouts** -- Entries do not expire (hardcoded to 0).
- **No ARP-based routing** -- Only supports local subnet communication.

## ICMP (Internet Control Message Protocol)

ICMP provides echo (ping) support for network diagnostics.

### ICMP Echo Request/Reply

When the kernel receives an ICMP echo request for its IP address (`192.168.100.2`):

1. The RX task parses the ICMP header.
2. Extracts the echo request payload (data to echo back).
3. Constructs an ICMP echo reply with the same payload.
4. Sends the reply back to the requester's MAC address (from the IP header).

### Ping Example

```bash
# From another machine on 192.168.100.0/24
$ ping 192.168.100.2
PING 192.168.100.2 (192.168.100.2) 56(84) bytes of data.
64 bytes from 192.168.100.2: icmp_seq=1 ttl=64 time=0.500ms
```

## Async Packet Reception

The network stack integrates with Hadron's async executor for non-blocking packet reception.

### NetworkRxTask

The `NetworkRxTask` runs as a critical-priority async task that continuously polls the NIC driver for incoming packets:

1. **Await NIC descriptor availability** -- Yield if the NIC's RX ring is empty.
2. **Receive packet** -- Call the NIC driver's `recv()` method (future).
3. **Parse Ethernet frame** -- Extract source MAC and EtherType.
4. **Dispatch to protocol handler**:
   - **EtherType 0x0806** (ARP) → Call `handle_arp_packet()`
   - **EtherType 0x0800** (IPv4) → Call `handle_ipv4_packet()`
5. **Loop** -- Return to step 1.

The task is marked as `Priority::Critical` so it runs with high priority, ensuring timely packet processing even if kernel services are busy.

### Zero-Copy Packet Handling

Packets are parsed in-place from the NIC driver's DMA buffer. No extra copying occurs until the response is constructed. For ICMP echo, the response payload is written directly from the request payload with minimal overhead.

## Phase 2 Roadmap

The following are deferred to Phase 2:

- **TCP and UDP** -- Transport layer protocols for data transmission.
- **Socket API** -- `socket()`, `bind()`, `connect()`, `send()`, `recv()` syscalls.
- **Routing** -- Multi-subnet support with configurable routes.
- **DNS** -- Domain name resolution (requires UDP).
- **DHCP** -- Dynamic IP address assignment.
- **MTU handling** -- Packet fragmentation for large packets.

## Implementation Status

IPv4 header parsing and validation
ARP request/reply handling
ARP table (16-entry cache)
ICMP echo request/reply
Async packet reception loop
NIC driver integration (VirtIO, e1000e)
TCP/UDP transport layer
Socket syscalls
Routing and multi-subnet support

## Files to Modify

- `kernel/kernel/src/net/mod.rs` -- Network configuration and initialization
- `kernel/kernel/src/net/ipv4.rs` -- IPv4 header parsing and dispatch
- `kernel/kernel/src/net/arp.rs` -- ARP table and resolution
- `kernel/kernel/src/net/icmp.rs` -- ICMP echo request/reply handling
- `kernel/kernel/src/net/task.rs` -- Async RX loop task
- `kernel/drivers/src/net/` -- NIC driver implementations (VirtIO, e1000e, Bochs)

## References

- **Architecture**: [Task Execution & Scheduling](../architecture/task-execution.md)
- **I/O & Filesystem**: [I/O & Filesystem](../architecture/io-filesystem.md)
- **Network Stack - Phase 2**: [Networking - TCP/UDP](../features/networking.md)
