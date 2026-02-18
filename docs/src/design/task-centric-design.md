# Task-Centric OS Design

This document defines Hadron's high-level design goals and the task-centric architecture that drives the kernel's future direction. It supersedes the incremental POSIX approach described in [POSIX Compatibility](posix-compatibility.md) and reframes the syscall strategy from [Syscall Strategy](syscall-strategy.md) around a native Hadron interface.

## Design Philosophy

Hadron is not a Linux clone. It is a general-purpose operating system that takes a fundamentally different approach to processes, security, and IPC — one that aligns with the strengths of the async executor architecture (Phase 6) and the framekernel's safety guarantees.

Three principles guide the design:

1. **Tasks are the universal primitive.** There is no distinction between processes, threads, kernel workers, and interrupt handlers at the scheduler level. They are all async tasks on the executor, differentiated only by their capabilities and address spaces.

2. **No ambient authority.** A task can only access resources it was explicitly given handles to. There is no "root user" that bypasses all checks, no world-readable files, no implicit inheritance of the full environment. This is capability-based security.

3. **Structured concurrency.** Tasks form a tree. Parents own their children. When a parent dies, its children are cleaned up. No zombies, no orphans, no reparenting to init. This mirrors Rust's ownership model at the OS level.

## Why Not POSIX?

POSIX is a 1970s process model extended with threads. It assumes:

- Processes are the unit of isolation (fork + exec, copy-on-write)
- File descriptors are the universal handle (but only for I/O, not for processes, memory, or events)
- Signals are the async notification mechanism (and they are unreliable, racy, and complex)
- Ambient authority via UID/GID determines access (any process running as root can do anything)
- `errno` is a thread-local global (requires TLS infrastructure)

These assumptions create enormous implementation complexity for limited benefit in a new system with no existing userspace. Hadron instead keeps what POSIX got right (hierarchical filesystem, byte-stream I/O, process isolation) while replacing the mechanisms that cause problems.

| POSIX Concept | Problem | Hadron Replacement |
|---------------|---------|-------------------|
| `fork()` | Copies entire address space; CoW page tables are complex; vfork/posix_spawn exist because fork is too expensive | `task_spawn` — creates a new task with a new address space directly |
| Signals | 64 signal numbers, signal masks, signal stacks, SA_RESTART, async-signal-safe functions, race conditions between sigaction and delivery | Events — waitable kernel objects, no handler reentrancy, no signal masks |
| `errno` | Thread-local global, requires TLS, easy to clobber | Return value encoding — errors returned directly from syscalls |
| UID/GID permissions | Ambient authority, root bypass, setuid complexity | Capability handles — explicit, unforgeable, fine-grained |
| File descriptors | Only covers files/sockets/pipes, not processes or memory regions | Handles — universal token for any kernel object |
| `select`/`poll`/`epoll` | Three incompatible APIs for the same thing, evolved over decades | `event_wait_many` — single unified multiplexing primitive |

## Core Primitives

### Tasks

A task is the universal unit of execution:

```
Task = async Future + HandleTable + optional AddressSpace
```

Every task has:
- A **TaskId** — unique identifier, packed with priority and CPU affinity into the waker encoding (already implemented in Phase 6)
- A **HandleTable** — the set of kernel object handles this task holds (its capabilities)
- An optional **AddressSpace** — if present, the task runs in ring 3 (userspace); if absent, it runs in kernel context

Different "kinds" of tasks emerge from different configurations, not from different kernel types:

| Configuration | Address Space | Handle Set | Analogous To |
|---------------|--------------|------------|--------------|
| Kernel task | None (kernel context) | Full kernel access | Kernel thread / workqueue |
| User task | Private | Inherited from parent | POSIX process |
| Shared task | Shared with parent | Shared handles | POSIX thread |
| Sandboxed task | Private | Minimal (parent-mediated) | Container / sandbox |
| Driver task | None (kernel context) | Device-specific | Kernel driver module |

The executor does not distinguish between these. It schedules them identically using the existing priority system (Critical, Normal, Background). The syscall layer and capability checks are what enforce the differences.

### Handles

A handle is an unforgeable, capability-checked token for a kernel object:

```rust
/// A handle is an index into a per-task handle table.
/// It is not a global identifier — handle 3 in task A
/// refers to a different object than handle 3 in task B.
pub struct Handle(u32);
```

Handle types:

| Type | Description | Replaces |
|------|-------------|----------|
| `Channel` | Bidirectional message-passing IPC | Pipes, Unix sockets, signals |
| `Vnode` | File or directory access | File descriptors (for files) |
| `MemoryObject` | Shared memory region | `shmget`/`shmat`, `mmap` shared |
| `TaskHandle` | Control handle to a child task | `pid_t` + `waitpid` + `kill` |
| `Event` | Waitable notification object | Signals, eventfd, futex wake |
| `Timer` | Periodic or one-shot timer | `setitimer`, `timer_create` |
| `Device` | Hardware device access | `/dev/*` + `ioctl` |

Key properties:

- **Unforgeable.** Tasks cannot fabricate handles. They can only receive them from the kernel (via syscalls that create objects) or from their parent (at spawn time or via channel transfer).
- **Transferable.** Handles can be sent over channels, allowing delegation of access. A task can give another task read access to a file by sending the vnode handle over a channel.
- **Revocable.** Closing a handle revokes the capability. If a parent revokes a handle it shared with a child, the child loses access.
- **Rights-bearing.** Each handle carries a rights mask (read, write, execute, duplicate, transfer). A task can create a reduced-rights copy of a handle before passing it to a child.

### Channels

Channels are the primary IPC mechanism, replacing pipes, signals, and local sockets:

```rust
pub struct Channel {
    /// Buffered messages waiting to be received.
    buffer: VecDeque<Message>,
    /// Waker for the receiving side.
    recv_waitqueue: HeapWaitQueue,
    /// Waker for the sending side (backpressure).
    send_waitqueue: HeapWaitQueue,
    /// Maximum buffered messages before backpressure.
    capacity: usize,
    /// Peer channel (the other end).
    peer: Option<Arc<Channel>>,
}
```

Properties:
- **Bidirectional.** Each `channel_create` syscall returns a pair of handles — one for each end.
- **Message-oriented.** Unlike POSIX pipes (byte streams), channels send discrete messages. Each message is a byte buffer plus an optional set of handles to transfer.
- **Backpressure.** When the buffer is full, `channel_send` blocks (or returns `WouldBlock` for async callers) until the receiver drains messages.
- **Handle transfer.** Messages can carry handles, enabling capability delegation. This is how a parent gives a child access to files, devices, or other tasks.
- **EOF semantics.** When one end is closed, the other end receives EOF on the next read. No SIGPIPE, no surprise termination.

### Events

Events replace POSIX signals with a simpler, race-free notification mechanism:

```rust
pub struct Event {
    /// Pending signal count (or bitfield for edge-triggered).
    pending: AtomicU64,
    /// Tasks waiting for this event.
    waitqueue: HeapWaitQueue,
}
```

Properties:
- **No handler reentrancy.** Events are polled, not delivered asynchronously into the middle of executing code. A task checks for events at well-defined points (syscall return, preemption return).
- **No signal masks.** No need to block/unblock events. If a task doesn't check for events, they accumulate.
- **Waitable.** Events integrate with `event_wait_many` for multiplexed waiting.
- **Composable.** Multiple events can be waited on simultaneously, including channel readiness, timer expiry, and child task completion.

## Task Lifecycle

### Spawning

```
Parent task
  |
  +-- task_spawn(binary, args, capabilities) -> TaskHandle
        |
        +-- Kernel loads ELF binary
        +-- Creates new AddressSpace (private PML4)
        +-- Creates new HandleTable with ONLY the explicitly passed capabilities
        +-- Spawns async task on executor at Priority::Normal
        +-- Returns TaskHandle to parent
```

The parent explicitly chooses which capabilities the child receives. There is no implicit inheritance of the full environment. If the parent wants the child to have access to a file, it must pass the vnode handle. If it wants the child to be able to write to the console, it must pass the console channel handle.

A minimal spawn:

```rust
// Parent creates a channel pair for the child's stdio
let (parent_end, child_end) = sys_channel_create()?;

// Parent spawns child with only the stdio channel and a vnode to its binary
let child = sys_task_spawn(
    binary_vnode,           // ELF to load
    &args,                  // argv
    &[child_end],           // handles to pass (child gets handle 0 = stdio)
)?;

// Parent communicates with child via parent_end
sys_channel_send(parent_end, b"hello from parent", &[])?;

// Parent waits for child to exit
let status = sys_task_wait(child)?;
```

### Structured Concurrency

Tasks form a supervision tree:

```
init (task 0)
 +-- display-server (task 1)
 |    +-- terminal (task 5)
 |    |    +-- shell (task 8)
 |    |         +-- ls (task 12)
 |    +-- status-bar (task 6)
 +-- network-manager (task 2)
 |    +-- dhcp-client (task 9)
 +-- filesystem-service (task 3)
```

Rules:
- When a parent exits, all its non-detached children receive a **kill event**.
- Children have a bounded grace period to clean up before forced termination.
- A parent can **detach** a child to a supervisor task, transferring ownership. This is how daemons work — `init` spawns a service, and the service detaches its workers to a supervisor.
- `task_wait` blocks until a specific child (or any child) exits, returning its exit status.
- No zombie tasks. When a child exits, its exit status is stored until the parent calls `task_wait`, then the task is fully cleaned up. If the parent exits without waiting, cleanup is immediate.

### Detachment and Supervisors

Long-running services need to outlive their parent. Hadron handles this with explicit detachment:

```rust
// Supervisor pattern: init spawns a service manager
let svc_mgr = sys_task_spawn(svc_manager_binary, &[], &[...])?;

// Service manager spawns services, then detaches them to itself
// (services outlive any individual request)
fn service_manager_main() {
    let db = sys_task_spawn(database_binary, &[], &[...])?;
    sys_task_detach(db, SELF)?;  // db is now owned by service_manager, not its caller

    let web = sys_task_spawn(webserver_binary, &[], &[db_channel])?;
    sys_task_detach(web, SELF)?;

    // Service manager monitors children via task_wait
    loop {
        let (child, status) = sys_task_wait_any()?;
        // Restart crashed services
        if status.is_crash() {
            restart(child);
        }
    }
}
```

## Native Syscall Interface

Hadron defines a small, orthogonal set of native syscalls. The total count targets roughly 30 syscalls for a complete general-purpose OS, compared to Linux's 450+.

### Task Management

| Syscall | Signature | Description |
|---------|-----------|-------------|
| `task_spawn` | `(binary: Handle, args: &[&str], handles: &[Handle]) -> TaskHandle` | Spawn a new task from an ELF binary, passing explicit capabilities |
| `task_exit` | `(status: i32) -> !` | Terminate the current task |
| `task_wait` | `(child: TaskHandle) -> ExitStatus` | Wait for a specific child to exit |
| `task_wait_any` | `() -> (TaskHandle, ExitStatus)` | Wait for any child to exit |
| `task_kill` | `(child: TaskHandle)` | Terminate a child task |
| `task_detach` | `(child: TaskHandle, new_parent: TaskHandle)` | Transfer child ownership to another task |
| `task_info` | `(task: TaskHandle) -> TaskInfo` | Query task status and metadata |

### Handle Operations

| Syscall | Signature | Description |
|---------|-----------|-------------|
| `handle_close` | `(h: Handle)` | Close a handle, revoking the capability |
| `handle_dup` | `(h: Handle, rights: Rights) -> Handle` | Duplicate with equal or reduced rights |
| `handle_info` | `(h: Handle) -> HandleInfo` | Query handle type and rights |

### Channels

| Syscall | Signature | Description |
|---------|-----------|-------------|
| `channel_create` | `() -> (Handle, Handle)` | Create a channel pair |
| `channel_send` | `(ch: Handle, data: &[u8], handles: &[Handle])` | Send a message, optionally transferring handles |
| `channel_recv` | `(ch: Handle, buf: &mut [u8]) -> (usize, Vec<Handle>)` | Receive a message and any transferred handles |
| `channel_call` | `(ch: Handle, data: &[u8]) -> (Vec<u8>, Vec<Handle>)` | Send + receive atomically (RPC pattern) |

### Filesystem (Vnodes)

| Syscall | Signature | Description |
|---------|-----------|-------------|
| `vnode_open` | `(dir: Handle, path: &str, flags: OpenFlags) -> Handle` | Open a file/directory relative to a directory handle |
| `vnode_read` | `(vn: Handle, buf: &mut [u8], offset: u64) -> usize` | Read from a vnode |
| `vnode_write` | `(vn: Handle, data: &[u8], offset: u64) -> usize` | Write to a vnode |
| `vnode_stat` | `(vn: Handle) -> VnodeStat` | Query file metadata |
| `vnode_readdir` | `(vn: Handle, buf: &mut [DirEntry]) -> usize` | List directory entries |
| `vnode_unlink` | `(dir: Handle, name: &str)` | Remove a file or directory |

Note: `vnode_open` takes a **directory handle** as its first argument, not a path string. This enforces capability-based access — you can only open files within directories you already hold a handle to. The root directory handle is passed to `init` by the kernel at boot.

### Memory

| Syscall | Signature | Description |
|---------|-----------|-------------|
| `mem_map` | `(addr: Option<usize>, size: usize, prot: Protection) -> *mut u8` | Map anonymous memory |
| `mem_unmap` | `(addr: *mut u8, size: usize)` | Unmap memory |
| `mem_protect` | `(addr: *mut u8, size: usize, prot: Protection)` | Change page protections |
| `mem_create_shared` | `(size: usize) -> Handle` | Create a shared memory object (transferable via channel) |
| `mem_map_shared` | `(obj: Handle, offset: u64, size: usize, prot: Protection) -> *mut u8` | Map a shared memory object into the address space |

### Events and Waiting

| Syscall | Signature | Description |
|---------|-----------|-------------|
| `event_create` | `() -> Handle` | Create a waitable event |
| `event_signal` | `(ev: Handle, value: u64)` | Signal an event with a value |
| `event_wait` | `(ev: Handle) -> u64` | Wait for an event to be signaled |
| `event_wait_many` | `(items: &mut [WaitItem]) -> usize` | Wait for any of several handles to become ready |

`event_wait_many` is the universal multiplexing primitive. A `WaitItem` specifies a handle and the events of interest (readable, writable, signaled, child exited). It replaces `select`, `poll`, and `epoll` with a single interface.

### Time

| Syscall | Signature | Description |
|---------|-----------|-------------|
| `clock_gettime` | `(clock: ClockId) -> Timespec` | Get current time (vDSO fast path) |
| `timer_create` | `(clock: ClockId, interval: Duration) -> Handle` | Create a periodic or one-shot timer |

### Async I/O (Optional Fast Path)

| Syscall | Signature | Description |
|---------|-----------|-------------|
| `io_ring_create` | `(entries: u32) -> Handle` | Create an I/O submission/completion ring |
| `io_submit` | `(ring: Handle, ops: &[IoOp])` | Submit I/O operations |
| `io_complete` | `(ring: Handle, buf: &mut [IoCompletion]) -> usize` | Poll for completed operations |

The async I/O ring is an opt-in fast path for programs that want to express concurrency directly to the kernel. It follows the io_uring model: userspace submits batches of operations and polls for completions without additional syscall transitions per operation. Programs that don't need this complexity use the standard blocking syscalls.

### System

| Syscall | Signature | Description |
|---------|-----------|-------------|
| `sys_query` | `(topic: u64, sub_id: u64, out_buf: *mut u8, out_len: usize) -> isize` | Query system information via typed `#[repr(C)]` response structs (memory, uptime, kernel version) |
| `sys_debug_log` | `(msg: &str)` | Write to kernel debug log (development only) |

## Userspace Programming Model

### Blocking by Default

The standard syscall interface is blocking. When a userspace task calls `vnode_read`, it blocks until data is available. Inside the kernel, this translates to:

1. Userspace calls `SYSCALL` with `vnode_read` arguments
2. Kernel entry, switch to kernel stack
3. `handle_syscall()` resolves the vnode handle, calls `inode.read().await`
4. If data is immediately available (ramfs, cached), the future resolves instantly and the syscall returns
5. If I/O is needed (disk, network), the future suspends. The kernel marks the userspace task as blocked and the executor picks up another task
6. When I/O completes, the waker fires, the task resumes, and the syscall returns to userspace

From the user program's perspective, `vnode_read` is a normal blocking call. The kernel's async machinery is invisible.

### Async-Native Extension

Programs that want to express concurrency directly (Rust async runtimes, event-driven servers) can use the I/O ring interface:

```rust
// Userspace async runtime using Hadron's io_ring
let ring = sys_io_ring_create(256)?;

// Submit multiple reads in one syscall
sys_io_submit(ring, &[
    IoOp::Read { handle: file_a, offset: 0, buf: buf_a.as_mut_ptr(), len: 4096 },
    IoOp::Read { handle: file_b, offset: 0, buf: buf_b.as_mut_ptr(), len: 4096 },
    IoOp::Recv { handle: socket, buf: net_buf.as_mut_ptr(), len: 1500 },
])?;

// Poll for completions
let n = sys_io_complete(ring, &mut completions)?;
for c in &completions[..n] {
    match c.op_index {
        0 => process_file_a(&buf_a[..c.result]),
        1 => process_file_b(&buf_b[..c.result]),
        2 => process_packet(&net_buf[..c.result]),
        _ => unreachable!(),
    }
}
```

This gives native Rust async programs a zero-overhead path to the kernel's async executor without the blocking-to-async-to-blocking round trip.

## Security Model

### Capability-Based Access Control

Hadron uses capability-based security instead of POSIX discretionary access control (DAC). The core principle is **no ambient authority**:

- A task cannot access any resource it was not explicitly given a handle to
- There is no "root" user that bypasses all permission checks
- There are no world-readable files — access requires a handle with read rights
- `vnode_open` requires a directory handle, not an absolute path string

### Rights Masks

Each handle carries a rights bitmask:

```rust
bitflags! {
    pub struct Rights: u32 {
        const READ      = 1 << 0;  // Can read data
        const WRITE     = 1 << 1;  // Can write data
        const EXECUTE   = 1 << 2;  // Can execute (for vnodes)
        const DUPLICATE = 1 << 3;  // Can dup this handle
        const TRANSFER  = 1 << 4;  // Can send this handle over a channel
        const MAP       = 1 << 5;  // Can mmap this object
        const STAT      = 1 << 6;  // Can query metadata
        const ENUMERATE = 1 << 7;  // Can list directory contents (for dir vnodes)
        const CREATE    = 1 << 8;  // Can create children (for dir vnodes)
        const DELETE    = 1 << 9;  // Can delete children (for dir vnodes)
        const SIGNAL    = 1 << 10; // Can signal this event/task
        const WAIT      = 1 << 11; // Can wait on this handle
    }
}
```

Rights can only be **reduced**, never elevated. A task can create a read-only copy of a read-write handle via `handle_dup(h, Rights::READ | Rights::STAT)`, but cannot add WRITE to a read-only handle.

### Capability Propagation

Capabilities flow through the task tree via two mechanisms:

1. **Spawn-time inheritance.** The parent explicitly passes handles in the `task_spawn` call. The child's handle table starts with exactly these handles and nothing else.

2. **Channel transfer.** A running task can send handles to another task over a channel. This enables dynamic delegation — a file server can grant access to a specific file by sending a vnode handle to a requesting client.

### Example: How a Shell Works

```
init
 +-- Holds: root_dir (vnode, full rights), console (channel)
 |
 +-- Spawns shell with: root_dir (read-only), console
      |
      +-- Shell receives handle 0 = root_dir (READ|STAT|ENUMERATE)
      +-- Shell receives handle 1 = console (READ|WRITE)
      |
      +-- User types "cat README.md"
      +-- Shell calls vnode_open(root_dir, "README.md", READ)
      |     -> Succeeds because shell has root_dir with ENUMERATE right
      |     -> Returns handle 2 = readme_vnode (READ|STAT)
      +-- Shell spawns cat with: readme_vnode (handle 0), console (handle 1)
      |     -> cat can read README.md and write to console
      |     -> cat CANNOT open other files (it has no directory handle)
      +-- cat reads file, writes to console, exits
      +-- Shell closes readme_vnode (handle 2)
```

This is more secure than POSIX by default. The `cat` program cannot access any file other than the one it was given. It cannot access the network, spawn other tasks (it has no binary vnodes), or read arbitrary files. A compromised `cat` is contained to its explicitly granted capabilities.

## Impact on Existing Phases

The task-centric model affects phases 7-15. Infrastructure phases 0-6 are unchanged.

| Phase | Original Plan | Revised Direction |
|-------|--------------|-------------------|
| **7: Syscall Interface** | Linux syscall numbers, POSIX stubs | Keep SYSCALL/SYSRET mechanism. Implement native Hadron syscall table. Linux-compat numbers can coexist for early testing. |
| **8: VFS & Ramfs** | Async VFS with file descriptor table | Async VFS with **handle table**. `vnode_open` takes a directory handle. Path resolution is relative to a capability, not absolute. |
| **9: Userspace** | Process struct with fd table, `exec()` | TaskContext with handle table and address space. `task_spawn` with explicit capability passing. |
| **10: Device Drivers** | PCI, VirtIO, async BlockDevice | Mostly unchanged. Device handles exposed to driver tasks. |
| **11: IPC & Signals** | Pipes, minimal signals, `sys_spawn`, `sys_waitpid` | **Channels and Events.** No signals. Channel-based IPC with handle transfer. Structured concurrency with `task_wait`/`task_kill`/`task_detach`. |
| **12: SMP** | Per-CPU executors, work stealing | Unchanged — executor scaling is independent of the task model. |
| **13: ext2** | Read-only ext2 | Unchanged (VFS backend). |
| **14: Networking** | VirtIO-net + smoltcp, socket syscalls | Socket handles instead of socket fds. Otherwise similar. |
| **15: vDSO** | vDSO + futex | vDSO for clock_gettime (unchanged). Events replace futex for userspace synchronization. |

## Comparison With Other Capability Systems

| Feature | Hadron | Fuchsia (Zircon) | seL4 | Capsicum (FreeBSD) |
|---------|--------|------------------|------|-------------------|
| Handle-based | Yes | Yes | Yes | Partial (capability mode) |
| Handle transfer over IPC | Yes (channels) | Yes (channels) | Yes (IPC endpoint) | No |
| No ambient authority | Yes | Yes | Yes | Opt-in (cap_enter) |
| Structured concurrency | Yes | No (jobs/processes) | No | No |
| Async executor scheduling | Yes | No (traditional threads) | No (traditional threads) | No |
| Runs in ring 0 (framekernel) | Yes | No (microkernel) | No (microkernel) | Yes (monolithic) |

Hadron is unique in combining capability-based security with an async executor and structured concurrency within a framekernel. Fuchsia (Zircon) is the closest comparison — it is also handle-based with channels — but it uses a microkernel architecture with traditional thread scheduling.

## Open Questions

These are areas where the design needs further refinement:

1. **Filesystem permissions model.** VFS vnodes need some form of access control metadata (who can the kernel grant handles to?). The capability model controls runtime access, but the *policy* of who gets what capabilities at spawn time needs a mechanism. One approach: an access control list (ACL) on each vnode checked at `vnode_open` time, consulted only by the VFS server task (which holds the root vnode with full rights).

2. **Dynamic linking and shared libraries.** How do dynamically linked programs work when `vnode_open` requires a directory handle? The loader needs a handle to the library directory. This could be a well-known handle index passed at spawn (e.g., handle 3 = library directory).

3. **Device driver sandboxing.** Driver tasks run in kernel context for performance, but could they be given limited capability sets to reduce the blast radius of a buggy driver? The framekernel's unsafe/safe split already provides memory safety; capabilities could additionally restrict which I/O ports or MMIO regions a driver can access.

4. **Debugging and introspection.** POSIX provides `ptrace` for debugging and `/proc` for introspection. Hadron replaces `/proc` with the typed `sys_query` syscall for kernel state queries, and will expose task introspection via `TaskHandle`-based syscalls (`sys_task_info`). A debug handle to a task could grant inspection rights without granting control.

5. **Resource limits.** POSIX uses `rlimit` for per-process resource limits. Hadron could attach resource budgets to tasks (memory quota, CPU time, handle count) enforced by the kernel, with budget inheritance/subdivision at spawn time.
