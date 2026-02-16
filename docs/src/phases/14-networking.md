# Phase 14: Networking

Major rewrite from the original plan. Instead of implementing a TCP/IP stack from scratch, this phase integrates the `smoltcp` crate to provide a production-quality network stack with minimal effort.

## Goal

Implement TCP/IP networking using a VirtIO-net driver and the smoltcp crate. Respond to ICMP pings, support TCP and UDP socket syscalls, and run a TCP echo server in userspace. The VirtIO-net driver implements smoltcp's `phy::Device` trait, and the network stack runs as an async kernel task.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `hadron-kernel/src/net/mod.rs` | Network subsystem: smoltcp `Interface` polling task |
| `hadron-kernel/src/net/socket.rs` | Socket abstraction wrapping smoltcp socket handles |
| `hadron-kernel/src/drivers/virtio/net.rs` | VirtIO-net driver implementing `smoltcp::phy::Device` |
| `hadron-kernel/src/syscall/net.rs` | Socket syscall handlers: socket, bind, connect, send, recv |
| `Cargo.toml` (hadron-kernel) | Add smoltcp dependency |

## Key Design

### smoltcp Integration

The smoltcp crate provides a complete, well-tested TCP/IP stack designed for embedded and OS use. It handles ARP, IPv4, ICMP, TCP, and UDP internally.

```toml
# hadron-kernel/Cargo.toml
[dependencies]
smoltcp = { version = "0.11", default-features = false, features = [
    "medium-ethernet",
    "proto-ipv4",
    "socket-tcp",
    "socket-udp",
    "socket-icmp",
]}
```

This eliminates the need to manually implement Ethernet framing, ARP resolution, IP checksums, and the TCP state machine. The kernel only needs to:

1. Provide a `phy::Device` implementation (VirtIO-net driver).
2. Poll the smoltcp `Interface` periodically or on events.
3. Map socket syscalls to smoltcp socket operations.

### VirtIO-net Driver

The VirtIO-net driver manages TX and RX virtqueues and implements smoltcp's physical device trait:

```rust
pub struct VirtioNet {
    rx_queue: VirtQueue,
    tx_queue: VirtQueue,
    mac: [u8; 6],
}

impl<'a> smoltcp::phy::Device for VirtioNet {
    type RxToken<'b> = VirtioRxToken<'b> where Self: 'b;
    type TxToken<'b> = VirtioTxToken<'b> where Self: 'b;

    fn receive(
        &mut self,
        _timestamp: smoltcp::time::Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.rx_queue.has_available() {
            Some((VirtioRxToken { queue: &mut self.rx_queue },
                  VirtioTxToken { queue: &mut self.tx_queue }))
        } else {
            None
        }
    }

    fn transmit(
        &mut self,
        _timestamp: smoltcp::time::Instant,
    ) -> Option<Self::TxToken<'_>> {
        Some(VirtioTxToken { queue: &mut self.tx_queue })
    }

    fn capabilities(&self) -> smoltcp::phy::DeviceCapabilities {
        let mut caps = smoltcp::phy::DeviceCapabilities::default();
        caps.max_transmission_unit = 1514;
        caps.medium = smoltcp::phy::Medium::Ethernet;
        caps
    }
}
```

### Network Stack as Async Task

The network stack runs as a kernel async task that polls smoltcp's `Interface`:

```rust
/// Main network polling loop, spawned as a kernel async task.
async fn net_poll_task(
    iface: &mut Interface,
    device: &mut VirtioNet,
    sockets: &mut SocketSet<'_>,
) {
    loop {
        let timestamp = smoltcp::time::Instant::from_millis(kernel_time_ms());
        iface.poll(timestamp, device, sockets);

        // Wake any tasks waiting on socket readiness
        notify_socket_waiters(sockets);

        // Sleep until next poll deadline or RX interrupt
        let delay = iface.poll_delay(timestamp, sockets);
        match delay {
            Some(duration) => sleep_or_irq(duration).await,
            None => wait_for_rx_irq().await,
        }
    }
}
```

### Socket Syscalls

Socket syscalls translate to smoltcp socket handle operations. When an operation returns `WouldBlock`, the syscall handler awaits a `WaitQueue` and retries:

```rust
pub async fn sys_recv(fd: usize, buf: UserPtr<u8>, len: usize) -> Result<usize, SyscallError> {
    let socket_handle = fd_to_socket_handle(fd)?;

    loop {
        let mut sockets = SOCKET_SET.lock();
        let socket = sockets.get_mut::<smoltcp::socket::tcp::Socket>(socket_handle);

        if socket.can_recv() {
            let n = socket.recv_slice(user_buf)
                .map_err(|_| SyscallError::ConnectionReset)?;
            return Ok(n);
        }

        if !socket.is_open() {
            return Err(SyscallError::ConnectionReset);
        }

        drop(sockets);
        // Yield until the net poll task signals data is available
        SOCKET_WAIT_QUEUES[fd].wait().await;
    }
}
```

### IRQ-Driven RX Notification

The VirtIO-net interrupt handler signals the network polling task on packet arrival, avoiding busy-wait polling:

```rust
fn virtio_net_irq_handler() {
    // Acknowledge interrupt
    virtio_net_device.ack_interrupt();
    // Wake the net_poll_task
    NET_RX_WAKER.wake();
}
```

## Key Data Structures

### Socket Wrapper

```rust
pub struct KernelSocket {
    pub handle: smoltcp::iface::SocketHandle,
    pub socket_type: SocketType,
    pub wait_queue: WaitQueue,
}

pub enum SocketType {
    Tcp,
    Udp,
    Icmp,
}
```

### Syscall Interface

| Syscall | Arguments | Description |
|---------|-----------|-------------|
| `socket` | domain, type, protocol | Create a smoltcp socket, return FD |
| `bind` | fd, addr, addrlen | Bind socket to local address/port |
| `connect` | fd, addr, addrlen | Initiate TCP connection |
| `listen` | fd, backlog | Mark socket as listening |
| `accept` | fd, addr, addrlen | Accept incoming TCP connection |
| `send` | fd, buf, len, flags | Send data on connected socket |
| `recv` | fd, buf, len, flags | Receive data from socket |
| `close` | fd | Close socket and release resources |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| VirtIO-net driver | Service | Uses safe VirtIO transport from Phase 10 |
| smoltcp `phy::Device` impl | Service | Adapter between VirtIO and smoltcp |
| Network poll task | Service | Async task using safe smoltcp APIs |
| Socket syscall handlers | Service | Map syscalls to smoltcp operations |
| smoltcp (ARP, IP, TCP, UDP) | Service | External crate, safe Rust |
| VirtIO-net IRQ handler | Service | Uses safe interrupt registration APIs |

The entire networking stack is safe service code. The VirtIO-net driver builds on the safe VirtIO transport abstraction from Phase 10.

## Milestone

**Verification**:

```
net: VirtIO-net online, MAC 52:54:00:12:34:56
net: interface configured, IP 10.0.2.15/24, gateway 10.0.2.2
net: ICMP echo reply sent to 10.0.2.2
```

From the host:
```bash
# Launch QEMU with user-mode networking
qemu-system-x86_64 ... \
    -netdev user,id=net0,hostfwd=tcp::5555-:7 \
    -device virtio-net,netdev=net0

# Ping the guest (via QEMU SLIRP)
ping 10.0.2.15

# Connect to TCP echo server running in guest userspace
echo "hello" | nc localhost 5555
# Output: hello
```

## Dependencies

- **Phase 8**: VFS (sockets exposed as file descriptors)
- **Phase 10**: VirtIO transport (device discovery, virtqueue management)
