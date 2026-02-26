# IPC Channels & Shared Memory

**Status: Completed** (implemented in commit 7e7deef)

Hadron provides two mechanisms for inter-process communication beyond pipes: typed async message channels for bounded message exchange, and shared memory regions for low-latency data sharing between processes.

Source: [`kernel/kernel/src/ipc/channel.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/ipc/channel.rs), [`kernel/kernel/src/ipc/shm.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/ipc/shm.rs)

## Channels

Bidirectional message channels enable processes to send fixed-size messages asynchronously. Each channel supports two endpoints (sender and receiver) with a 16-message circular buffer, and both endpoints can send or receive messages in either direction.

### Key Types

| Type | Role |
|------|------|
| `ChannelInner` | Shared state: two message queues (one per direction), wait queues for blocking |
| `ChannelEndpoint` | Sender or receiver handle; wraps `Arc<ChannelInner>` |

### Message Format

- **Max size**: 4 KiB per message
- **Buffer capacity**: 16 messages per direction
- **Blocking semantics**: Both send and receive yield to the async executor when buffers are full/empty

### Syscall Interface

- **`sys_channel_create()`** -- Creates a bidirectional channel. Returns two endpoint IDs (one for each endpoint).
- **`sys_channel_send(endpoint_id, msg_ptr, msg_len)`** -- Sends a message (up to 4 KiB). Blocks via `HeapWaitQueue` if the buffer is full and resumes when the receiver reads.
- **`sys_channel_recv(endpoint_id, msg_ptr, msg_len)`** -- Receives a message from the endpoint. Blocks if the buffer is empty and resumes when the sender writes.
- **`sys_channel_close(endpoint_id)`** -- Closes an endpoint. Sends should return an error to indicate the receiver is gone.

### Design Decisions

- **Bidirectional**: Unlike Unix pipes (one-way), both endpoints can send and receive, allowing peer-to-peer communication without creating two separate channels.
- **Async futures**: Both send and receive return pinned boxed futures that yield to the async executor when blocked, enabling efficient context switching.
- **Fixed message size**: 4 KiB is a reasonable trade-off for stack-allocatable messages during syscall handling without requiring unbounded heap allocation for each message.
- **Waker-based blocking**: Uses `HeapWaitQueue` to store wakers from blocked tasks, enabling FIFO ordering of multiple blocked senders/receivers.

### Implementation Status

Channel creation and basic send/receive
Async blocking with executor integration
Bidirectional messaging
Channel closure signaling (WIP)

## Shared Memory

Shared memory regions enable low-latency inter-process data exchange by mapping the same physical pages into multiple address spaces. Changes made in one process are immediately visible in others without data copying.

### Syscall Interface

- **`sys_mem_create_shared(size)`** -- Creates a shared memory region of the specified size. Returns an SHM ID handle that can be shared between processes.
- **`sys_mem_map_shared(shm_id, addr, perms)`** -- Maps a shared memory region into the current process's address space at the specified virtual address with the given permissions (read-only or read-write).
- **`sys_mem_unmap_shared(addr)`** -- Unmaps a previously mapped shared memory region.

### Key Features

- **Reference-counted lifecycle**: Shared memory regions are freed when the last process unmaps them and the last file descriptor is closed.
- **Physical page sharing**: The same physical pages are mapped into multiple address spaces, enabling true zero-copy sharing.
- **Permission control**: Each mapping can specify independent permissions (read-only vs read-write), allowing fine-grained access control.

### Use Cases

- **Zero-copy IPC**: Pass large buffers between processes without copying.
- **Shared state**: Implement lock-free data structures or shared caches across processes.
- **Frame buffers**: Share framebuffer memory with userspace compositor and display clients.

### Implementation Status

Shared memory region creation
Mapping into multiple address spaces
Reference counting and cleanup
Permission control per mapping

## Async Integration

Both channels and shared memory integrate with Hadron's cooperative async executor:

- **Blocking send/recv**: When a send would block (buffer full) or recv would block (buffer empty), the future yields to the async executor via `.await`, allowing other tasks to make progress.
- **Waker-based notification**: When the opposite endpoint performs an operation, blocked tasks are woken via their stored wakers, resuming execution on the next executor poll.
- **No spin-waiting**: Unlike spin locks, blocked channel operations don't burn CPU time; they genuinely yield to other tasks.

## Files to Modify

The following files implement this feature:

- `kernel/kernel/src/ipc/channel.rs` -- Channel creation, send/recv logic
- `kernel/kernel/src/ipc/shm.rs` -- Shared memory region management
- `kernel/kernel/src/syscall/ipc.rs` -- Syscall handlers for channel and SHM operations
- `kernel/syscall/src/lib.rs` -- Syscall DSL definitions for channel and memory syscalls

## References

- **Design**: [Synchronization & IPC](../architecture/sync-ipc.md)
- **Syscall Interface**: [Task Execution & Scheduling](../architecture/task-execution.md#syscall-interface)
- **Async Primitives**: [Task Execution & Scheduling](../architecture/task-execution.md#async-primitives)
