# Unix Domain Sockets

## Goal

Implement `AF_UNIX` stream sockets with fd-passing (`SCM_RIGHTS`) to provide
standard IPC transport for Wayland, Mesa, and general userspace tools. This is
the single most complex kernel addition in the graphics stack path, but it is
broadly useful and avoids patching every Wayland and Mesa transport layer.

## Background

Wayland compositors and clients communicate over Unix domain sockets. The Wayland
wire protocol uses `sendmsg`/`recvmsg` with `SCM_RIGHTS` ancillary data to pass
file descriptors (for shared memory buffers and DMA-buf handles) between
processes. Mesa's WSI layer connects to the compositor via
`$XDG_RUNTIME_DIR/wayland-0`.

Hadron already has the building blocks: an fd table per process, fd-passing via
`channel_send_fd`/`channel_recv_fd`, and a VFS with path-based addressing.
Unix domain sockets unify these into the standard POSIX socket API.

## Key Design

### Scope

Only `AF_UNIX` with `SOCK_STREAM` (connection-oriented byte stream). No
`SOCK_DGRAM`, no `SOCK_SEQPACKET`, no network sockets (`AF_INET`). This is the
minimum needed for Wayland and covers the vast majority of local IPC use cases.

### Socket Lifecycle

```
Server:                          Client:
  socket(AF_UNIX, SOCK_STREAM)     socket(AF_UNIX, SOCK_STREAM)
  bind("/run/wayland-0")           connect("/run/wayland-0")
  listen(backlog)                        |
  accept() ←─────────────────────────────┘
       |                                 |
  sendmsg/recvmsg  ←─────────────→  sendmsg/recvmsg
       |                                 |
  close()                           close()
```

### Kernel Representation

```rust
/// A Unix domain socket endpoint.
pub struct UnixSocket {
    /// Socket state machine.
    state: SpinLock<SocketState>,
    /// Bound path in VFS (if bound).
    bound_path: Option<String>,
    /// Connected peer (bidirectional byte stream + fd queue).
    peer: Option<Arc<UnixStreamPair>>,
    /// Pending connections (listen state).
    backlog: Option<ArrayQueue<Arc<UnixStreamPair>>>,
    /// Waker for poll_readiness.
    waker: AtomicWaker,
}

enum SocketState {
    Unbound,
    Bound,
    Listening { backlog: usize },
    Connected,
    Closed,
}

/// Bidirectional connected pair with byte buffers and fd queues.
pub struct UnixStreamPair {
    a_to_b: StreamBuffer,
    b_to_a: StreamBuffer,
}

pub struct StreamBuffer {
    bytes: RingBuffer<u8>,
    /// Ancillary fd queue: (byte_offset, Vec<RawFd>).
    /// fds are delivered with the byte at byte_offset.
    pending_fds: VecDeque<(usize, Vec<RawFd>)>,
    waker: AtomicWaker,
}
```

### Filesystem Integration

`bind()` creates a socket inode in the VFS at the given path. `connect()` looks
up the path via the VFS, finds the listening socket, and creates a connected
pair. The socket inode implements `Inode` so it participates in the existing
VFS namespace — `unlink` removes the socket path, `stat` reports `S_IFSOCK`.

### fd-Passing via SCM_RIGHTS

`sendmsg` with `SCM_RIGHTS` ancillary data transfers file descriptors from the
sender's fd table to the receiver's. The kernel:

1. Validates each fd in the sender's table.
2. Clones the `Arc<dyn FileHandle>` for each fd.
3. Enqueues the cloned handles in the stream's `pending_fds` queue, tagged with
   the byte offset at which they should be delivered.
4. On `recvmsg`, the receiver gets the fds installed into its fd table at the
   lowest available slots.

This reuses the same `Arc<dyn FileHandle>` cloning that `channel_send_fd`
already performs.

### Syscalls

| Syscall | Description |
|---------|-------------|
| `sys_socket(domain, type, protocol)` | Create socket fd (`AF_UNIX`, `SOCK_STREAM`) |
| `sys_bind(fd, path)` | Bind to filesystem path |
| `sys_listen(fd, backlog)` | Mark as listening, set backlog |
| `sys_accept(fd)` | Accept pending connection, return new fd |
| `sys_connect(fd, path)` | Connect to a listening socket |
| `sys_sendmsg(fd, msg, flags)` | Send bytes + optional `SCM_RIGHTS` fds |
| `sys_recvmsg(fd, msg, flags)` | Receive bytes + optional fds |
| `sys_shutdown(fd, how)` | Half-close (SHUT_RD, SHUT_WR, SHUT_RDWR) |

`send`/`recv` (without ancillary data) can be implemented as thin wrappers over
`sendmsg`/`recvmsg`, or as `write`/`read` on the socket fd.

### Poll Integration

Sockets implement `Inode::poll_readiness` for use with `event_wait_many`:

- Listening socket: readable when a pending connection exists.
- Connected socket: readable when bytes are available, writable when buffer has space.
- Readable when peer has closed (returns 0 on read, like EOF).

## Files to Create/Modify

| File | Description |
|------|-------------|
| `kernel/kernel/src/net/unix.rs` | **New:** Unix socket implementation |
| `kernel/kernel/src/net/mod.rs` | **New:** Socket subsystem entry point |
| `kernel/kernel/src/syscall/net.rs` | **New:** Socket syscall handlers |
| `kernel/syscall/src/lib.rs` | Add socket syscall numbers |
| `kernel/fs/src/lib.rs` | Add `S_IFSOCK` inode type |
| `hadron-libc/src/socket.c` | Implement `socket`/`bind`/`listen`/`accept`/`connect`/`sendmsg`/`recvmsg` |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| Socket state machine | Service | Pure Rust state transitions |
| Stream buffers | Service | Ring buffer management |
| fd-passing (SCM_RIGHTS) | Service | Arc cloning, fd table insertion |
| VFS socket inode | Service | Implements existing Inode trait |
| Syscall argument validation | Frame | User pointer validation, fd lookup |

## Dependencies

- **Async VFS & Ramfs**: Inode trait, path resolution (complete).
- **IPC Channels**: fd-passing infrastructure (complete).
- **hadron-libc**: C socket API wrappers (new work).

## Milestone

```
[compositor] bind: /run/wayland-0
[compositor] listening for clients

[client] connect: /run/wayland-0
[compositor] accepted client (pid 42)

[client] wl_display: connected, server version 1
[client] wl_registry: got wl_compositor, wl_shm, xdg_wm_base
[client] sendmsg: shared 4 fds (shm pool)
[compositor] recvmsg: received 4 fds from client
```
