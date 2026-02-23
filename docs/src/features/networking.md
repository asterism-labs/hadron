# Networking — TCP/UDP

> **Revised** — The smoltcp approach was dropped in favour of a custom network stack. ARP and ICMP over IPv4 are already implemented; this feature adds TCP and UDP on top of that foundation.

## Current State

The kernel already has a custom network stack with:

- **ARP** — address resolution protocol, cache management
- **ICMP** — echo request/reply (ping)
- **IPv4** — packet construction, routing, checksum
- **VirtIO-net** and **e1000e** drivers for Ethernet frame TX/RX
- **Async RX loop** — IRQ-driven packet reception

These live in `hadron-kernel/src/net/` and were implemented during earlier development.

## Goal

Build TCP and UDP transport protocols on the existing custom ARP/ICMP/IPv4 stack. Add socket syscalls so userspace programs can create network connections. After this feature, a userspace TCP echo server and UDP DNS client are possible.

## Key Design

### TCP Implementation

A minimal but correct TCP state machine:

```rust
pub struct TcpSocket {
    state: TcpState,
    local_addr: SocketAddr,
    remote_addr: Option<SocketAddr>,
    tx_buffer: CircularBuffer,
    rx_buffer: CircularBuffer,
    /// Sequence numbers
    snd_una: u32,   // Oldest unacknowledged
    snd_nxt: u32,   // Next to send
    rcv_nxt: u32,   // Next expected
    rcv_wnd: u16,   // Receive window
    /// Retransmission
    rto: Duration,
    retransmit_queue: Vec<TcpSegment>,
    /// Async wakeup
    rx_wq: WaitQueue,
    tx_wq: WaitQueue,
    connect_wq: WaitQueue,
    accept_wq: WaitQueue,
}

pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    LastAck,
    TimeWait,
    Closing,
}
```

The TCP implementation handles:
- Three-way handshake (active and passive open)
- Sliding window flow control
- Retransmission with exponential backoff
- Connection teardown (FIN/ACK)
- RST handling

### UDP Implementation

```rust
pub struct UdpSocket {
    local_addr: SocketAddr,
    rx_buffer: VecDeque<(SocketAddr, Vec<u8>)>,
    rx_wq: WaitQueue,
}
```

UDP is connectionless — `send` constructs and transmits a UDP datagram immediately; `recv` awaits incoming datagrams via a WaitQueue.

### Socket Syscalls

| Syscall | Description |
|---------|-------------|
| `sys_socket(domain, type, protocol)` | Create a TCP or UDP socket, return FD |
| `sys_bind(fd, addr, addrlen)` | Bind socket to local address/port |
| `sys_connect(fd, addr, addrlen)` | Initiate TCP connection (async — awaits handshake) |
| `sys_listen(fd, backlog)` | Mark TCP socket as listening |
| `sys_accept(fd, addr, addrlen)` | Accept incoming TCP connection (async — awaits SYN) |
| `sys_send(fd, buf, len, flags)` | Send data on connected socket |
| `sys_recv(fd, buf, len, flags)` | Receive data from socket (async — awaits data) |

All blocking operations are async — they await WaitQueues and yield to the executor, consistent with the kernel's async-everywhere model.

### Integration with Existing Stack

```
┌─────────────────────────────┐
│  Userspace (socket syscalls) │
├─────────────────────────────┤
│  Socket layer (TCP/UDP)      │  ← This feature
├─────────────────────────────┤
│  IPv4 + ICMP + ARP           │  ← Already implemented
├─────────────────────────────┤
│  VirtIO-net / e1000e         │  ← Already implemented
└─────────────────────────────┘
```

Incoming packets flow: NIC IRQ → driver RX → IPv4 dispatch → TCP/UDP demux → socket buffer → wake waiting task.

Outgoing packets flow: socket send → TCP/UDP → IPv4 → ARP resolve → driver TX.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `hadron-kernel/src/net/tcp.rs` | **New:** TCP state machine, segment handling |
| `hadron-kernel/src/net/udp.rs` | **New:** UDP socket implementation |
| `hadron-kernel/src/net/socket.rs` | **New:** Socket abstraction, port allocation |
| `hadron-kernel/src/net/mod.rs` | Update IPv4 dispatch to route to TCP/UDP |
| `hadron-kernel/src/syscall/net.rs` | **New:** Socket syscall handlers |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| TCP state machine | Service | Protocol logic, no hardware access |
| UDP socket | Service | Simple datagram buffering |
| Socket syscall handlers | Service | Map syscalls to socket operations |
| IPv4 dispatch (update) | Service | Routing table lookup, packet forwarding |
| Port allocation table | Service | Data structure management |

The entire feature is safe service code building on the existing network stack.

## Dependencies

- **Async VFS & Ramfs**: VFS (sockets exposed as file descriptors).
- **Device Drivers**: Network drivers (VirtIO-net, e1000e — already complete).
- Existing `hadron-kernel/src/net/` ARP/ICMP/IPv4 implementation.

## Milestone

```
tcp: listening on 0.0.0.0:7 (echo server)
tcp: connection from 10.0.2.2:54321 -> ESTABLISHED
tcp: echoed 13 bytes
tcp: connection closed (FIN)

udp: bound to 0.0.0.0:53
udp: sent DNS query to 10.0.2.3
udp: received DNS response: hadron.local -> 10.0.2.15
```

From the host:
```bash
# TCP echo test
echo "Hello, Hadron!" | nc localhost 5555

# UDP test
dig @localhost -p 5353 hadron.local
```
