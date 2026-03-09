# IPC: Channels, Ports, and Other Primitives

Inter-process communication in Hadron is capability-mediated: a process can only communicate with another if it holds a handle to an IPC object that connects them. This chapter covers all IPC object types: Channel, Port, Socket, Event, EventPair, Timer, and Fifo.

## Channel

A channel is the primary IPC mechanism. It is a bidirectional pipe that carries discrete messages, where each message consists of a byte payload and an optional list of transferred handles.

### Structure

A channel is created as a peer pair. `channel_create` returns two handles, one for each endpoint. Either endpoint can read and write; the messages travel in both directions independently (each direction has its own queue).

The channel object holds:

- A `Koid` for itself and a `peer_koid` for the other endpoint.
- An inbound message queue (messages sent by the peer, waiting to be read).
- A `Weak` reference to the peer endpoint (to detect peer closure without a strong cycle).
- A wait queue for threads blocked in `channel_read`.
- A `Signals` atomic tracking `READABLE`, `WRITABLE`, and `PEER_CLOSED`.
- An observer list for port notifications.

### ChannelMessage

```
ChannelMessage {
    data:    Vec<u8>,     // max 65536 bytes (64 KiB)
    handles: Vec<HandleEntry>,  // max 64 handles
}
```

The data limit (64 KiB) and handle limit (64) are per-message. There is no limit on the number of messages queued in a channel, subject to available kernel memory.

### channel_create

```
channel_create() -> (HandleValue, HandleValue)
```

Creates a channel pair. Both returned handles have `Rights::CHANNEL_DEFAULT`. The two endpoints are symmetric: either end can be given to a different process.

### channel_write

```
channel_write(channel: HandleValue, data: &[u8], handles: &[HandleValue]) -> Result<()>
```

Required rights on `channel`: `Rights::WRITE`.

Behavior:
1. Validates `data.len() <= 64 KiB` and `handles.len() <= 64`.
2. Removes each named handle from the caller's handle table (they are moved into the message).
3. Enqueues the `ChannelMessage` on the peer endpoint's inbound queue.
4. Asserts `READABLE` on the peer endpoint.
5. Wakes any thread waiting in `channel_read` on the peer.

If the peer endpoint has been closed (no remaining handles to it), returns `PEER_CLOSED` error.

### channel_read

```
channel_read(channel: HandleValue, data_buf: &mut [u8], handle_buf: &mut [HandleValue])
    -> Result<(usize, usize)>
```

Required rights: `Rights::READ`.

Behavior:
1. Dequeues the next message from the inbound queue (blocks if the queue is empty).
2. Copies `data` into `data_buf`.
3. Inserts each transferred handle into the caller's handle table, writing the new `HandleValue` integers into `handle_buf`.
4. Returns `(bytes_read, handles_read)`.

If the queue becomes empty after the read, clears the `READABLE` signal.

### channel_call (RPC Pattern)

`channel_call` combines a write and a blocking read into a single atomic operation, enabling synchronous RPC:

```
channel_call(channel: HandleValue, request: ChannelMessage, deadline: Instant)
    -> Result<ChannelMessage>
```

Required rights: `Rights::READ | Rights::WRITE`.

The kernel:
1. Writes the request message to the peer.
2. Blocks the calling thread until a reply arrives on the same endpoint (or deadline expires).
3. Returns the reply message.

```mermaid
sequenceDiagram
    participant Client
    participant KernelChannel as Channel (kernel)
    participant Server

    Client->>KernelChannel: channel_call(request)
    Note over KernelChannel: enqueue request on Server's inbound queue
    KernelChannel->>Server: READABLE signal (via port)
    Server->>KernelChannel: channel_read() -> request
    Server->>Server: process request
    Server->>KernelChannel: channel_write(reply)
    Note over KernelChannel: enqueue reply on Client's inbound queue
    KernelChannel-->>Client: channel_call returns reply
    Note over Client: thread was blocked; now resumes
```

`channel_call` is the idiomatic pattern for synchronous request-response interactions between a client process and a service. It does not require the client to manage a separate port for the reply.

## Port

A port is an async event aggregation object, analogous to `epoll` on Linux. A thread registers interest in signals on multiple objects, then blocks in a single `port_wait` call. When any registered object fires a signal, a `PortPacket` is delivered to the port.

### object_wait_async

```
object_wait_async(object: HandleValue, port: HandleValue, key: u64, signals: Signals)
    -> Result<()>
```

Required rights: `Rights::WAIT` on `object`; `Rights::WRITE` on `port`.

Registers the port as an observer on `object` for the given `signals`. `key` is a caller-supplied u64 that is echoed back in the `PortPacket` — it is typically used to identify which object fired.

Multiple objects can be registered on the same port. A single object can be registered on multiple ports. Registering the same (object, port) pair a second time replaces the previous registration.

### port_wait

```
port_wait(port: HandleValue, deadline: Instant) -> Result<PortPacket>
```

Required rights: `Rights::READ` on `port`.

Blocks the calling thread until a `PortPacket` is available in the port's queue, or `deadline` expires. Returns `TIMED_OUT` if no packet arrives by the deadline.

A typical server loop:

```rust
loop {
    let packet = port_wait(port, Instant::INFINITE)?;
    match packet.key {
        KEY_LISTENER => handle_new_connection(packet),
        KEY_CLIENT_N => handle_client_data(packet),
        _ => {}
    }
}
```

### PortPacket

```
PortPacket {
    key:     u64,
    signals: Signals,
    koid:    Koid,      // koid of the object that fired
    type:    PacketType,
    // type-specific payload (timer expiry time, interrupt vector, etc.)
}
```

## Socket

A socket is a streaming byte IPC primitive, analogous to a Unix socket pair. Unlike channels, sockets carry an undifferentiated byte stream (no message boundaries) and do not transfer handles.

Created as a peer pair (`socket_create`). Each endpoint has a read buffer and a write buffer. Writing to one endpoint fills the peer's read buffer.

Signals:
- `READABLE` — data is available to read from this endpoint.
- `WRITABLE` — the peer's read buffer has space.
- `PEER_CLOSED` — the peer endpoint has been closed.

Sockets are appropriate for bulk streaming data (e.g., a terminal's byte stream) where message framing is not needed and handle transfer is not required.

## Event

An event is the simplest signaling primitive. It has no data; it exists solely to carry user-visible signals `SIGNAL_0` through `SIGNAL_4`.

```
event_create() -> HandleValue
```

The creator can assert or clear signal bits with `object_signal`. Other processes that hold a handle to the event (with `Rights::WAIT`) can observe signal changes via a port or a direct blocking wait.

Events are used for one-shot notifications (e.g., "initialization complete") and for simple synchronization between processes.

## EventPair

An event pair is a linked pair of events. Each endpoint can set signals on itself that are visible to the peer. When one endpoint is closed, `PEER_CLOSED` is asserted on the surviving peer.

```
eventpair_create() -> (HandleValue, HandleValue)
```

EventPairs are used for bidirectional rendezvous signaling. For example, a compositor and a client can use an EventPair to signal frame availability and frame consumption.

## Timer

A timer object fires at a specified deadline. When the deadline arrives, the kernel asserts `SIGNAL_0` on the timer. Timers are almost always used with a port: register the timer with a port via `object_wait_async`, then observe the timer firing in the port loop.

```
timer_create() -> HandleValue
timer_set(timer: HandleValue, deadline: Instant, slack: Duration) -> Result<()>
timer_cancel(timer: HandleValue) -> Result<()>
```

The `slack` parameter allows the kernel to coalesce nearby timer firings, reducing unnecessary wakeups. A slack of zero requests exact firing.

Timers are one-shot by default. To create a periodic timer, call `timer_set` again from within the port handling code after the previous firing.

## Fifo

A Fifo is a fixed-element-size queue designed for high-frequency, low-latency messaging between exactly two processes. Unlike channels, Fifos:

- Have a fixed element size determined at creation time.
- Have a fixed capacity (power of two, set at creation).
- Do not transfer handles.
- Use a lock-free ring buffer internally.

```
fifo_create(elem_count: u32, elem_size: u32) -> (HandleValue, HandleValue)
fifo_write(fifo: HandleValue, data: &[u8]) -> Result<usize>
fifo_read(fifo: HandleValue, buf: &mut [u8]) -> Result<usize>
```

The `data` slice for `fifo_write` must be a multiple of `elem_size` bytes. The call writes as many complete elements as fit in the remaining capacity and returns the count written.

Fifos are appropriate for driver-to-userspace data paths where channel overhead (heap allocation per message) is unacceptable. For example, a NIC driver can push received packet metadata through a Fifo, while the actual packet data lives in a shared VMO.

Signals:
- `READABLE` — at least one element is available to read.
- `WRITABLE` — at least one slot is available to write.

## Choosing the Right IPC Primitive

| Primitive | Direction | Data | Handle transfer | Use when |
|-----------|-----------|------|-----------------|----------|
| Channel | Bidirectional | Framed messages up to 64 KiB | Yes (up to 64) | General-purpose RPC, capability delegation |
| Socket | Bidirectional | Byte stream, unbounded | No | Terminal I/O, bulk streaming |
| Fifo | Bidirectional | Fixed-size elements | No | High-frequency, low-latency data paths |
| Port | Receive-only | PortPackets from observers | No | Async event multiplexing (server loops) |
| Event | Unidirectional | Signals only | No | Simple notifications |
| EventPair | Bidirectional | Signals only | No | Paired rendezvous signaling |
| Timer | Unidirectional | Signals only | No | Deadline-based wakeup |
