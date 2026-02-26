# Synchronization & Inter-Process Communication

Hadron provides a layered set of synchronization primitives and IPC mechanisms. Synchronization primitives enable safe concurrent access to kernel data structures, while IPC primitives allow processes to exchange data and coordinate execution. Both categories are designed to be `const`-constructable so they can live in `static` items and be usable before the heap allocator or async executor is available.

## Synchronization Primitives

Source: [`kernel/kernel/src/sync/`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/sync/)

Synchronization primitives fall into three categories: spin-based (no heap/executor), async-aware (uses executor), and initialization.

| Category | Primitives | Requires heap | Requires executor |
|---|---|---|---|
| Spin-based | `SpinLock`, `IrqSpinLock`, `RwLock` | No | No |
| Async-aware | `Mutex`, `WaitQueue`, `HeapWaitQueue` | Partial | Yes |
| Initialization | `LazyLock` | No | No |

### SpinLock

A basic mutual-exclusion lock that busy-waits until the lock is available. Uses test-and-test-and-set (TTAS) to reduce cache-line contention: the acquire loop first attempts a `compare_exchange_weak` on the `AtomicBool`, then falls back to spinning on a relaxed load until the lock appears free.

**API:**
- `SpinLock::new(value)` -- const constructor.
- `lock() -> SpinLockGuard` -- spins until acquired.
- `try_lock() -> Option<SpinLockGuard>` -- single non-blocking attempt.
- `unsafe force_get() -> &mut T` -- bypasses the lock entirely (panic path escape hatch).

Use `SpinLock` for short critical sections in non-interrupt code paths where the protected data does not need to be accessed from interrupt handlers.

### IrqSpinLock

A spin lock that disables interrupts before acquiring the inner lock and restores the previous interrupt state on release. This prevents deadlock when an interrupt handler tries to acquire a lock that the interrupted code already holds.

**Interrupt state management:**

On x86_64, the guard saves the `RFLAGS` register (via `pushfq; pop`) before executing `cli`. On drop, it checks the saved interrupt flag (bit 9) and only executes `sti` if interrupts were previously enabled. This means nested acquisitions work correctly: the inner lock's drop does not re-enable interrupts if the outer lock had already disabled them.

The guard explicitly implements `!Send` because interrupt state is per-CPU, so migrating a guard to another CPU would be unsound.

**Use IrqSpinLock** whenever the protected data may be accessed from both normal kernel code and interrupt handlers.

### RwLock

A spinning reader-writer lock that allows multiple concurrent readers or a single exclusive writer. The lock state is packed into a single `AtomicU32`:

| Value | Meaning |
|---|---|
| `0` | Unlocked |
| `1..u32::MAX-1` | Read-locked with N active readers |
| `u32::MAX` | Write-locked |

**Use RwLock** for data that is read frequently but written rarely, such as routing tables or configuration state.

### WaitQueue

A fixed-capacity queue of `Waker`s (capacity 32) for interrupt-driven wakeups. Tasks register their waker by calling `wait()` or `register_waker()`. Interrupt handlers call `wake_one()` or `wake_all()` to resume waiting tasks.

The internal storage is a `planck_noalloc::ArrayVec<Waker, 32>` protected by an `IrqSpinLock`, so `WaitQueue` is usable before the heap allocator is available and is safe to wake from interrupt context.

**API:**
- `WaitQueue::new()` -- const constructor.
- `wait() -> WaitFuture` -- returns a future that pends on the first poll and completes on the second.
- `register_waker(&Waker) -> bool` -- manually register a waker; returns `false` if full.
- `wake_one()` -- wakes the first registered waker (FIFO).
- `wake_all()` -- drains all wakers and wakes each outside the critical section.

**Use WaitQueue** for frame-layer primitives where capacity is bounded (the executor itself, low-level I/O completion).

### HeapWaitQueue

A heap-backed wait queue with unbounded capacity, using `alloc::collections::VecDeque<Waker>` for O(1) FIFO `wake_one()` via `pop_front()`.

**API mirrors WaitQueue:**
- `HeapWaitQueue::new()` -- const constructor.
- `wait() -> HeapWaitFuture` -- future-based waiting.
- `register_waker(&Waker)` -- always succeeds (no capacity limit).
- `wake_one()` / `wake_all()` -- same semantics as WaitQueue.

**Use HeapWaitQueue** for service-layer primitives (channels, barriers, condition variables) where the number of concurrent waiters is unpredictable.

### Mutex

An async-aware mutual-exclusion lock. Unlike `SpinLock`, a contended `Mutex` yields the current task back to the executor via a `WaitQueue` rather than busy-waiting.

**MutexLockFuture::poll** uses a careful protocol to avoid lost wakeups:

1. **Fast path:** attempt `compare_exchange_weak`. If it succeeds, return `Poll::Ready`.
2. **Register waker:** call `WaitQueue::register_waker` with the task's `Waker`.
3. **Retry:** attempt acquisition again (catches race between steps 1 and 2).
4. **Fallback:** if the `WaitQueue` is full, self-wake and degrade to spin-poll.

**Use Mutex** for critical sections in async task code where the lock may be held across `.await` points or where contention is expected.

### LazyLock

A `no_std` equivalent of `std::sync::LazyLock` that initializes a value on first access. Uses an atomic state machine with four states: `UNINIT`, `INITIALIZING`, `READY`, and `POISONED`.

## Inter-Process Communication

Source: [`kernel/kernel/src/ipc/`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/ipc/)

Hadron provides two levels of IPC:

1. **Pipes** -- byte-oriented streams integrated with the VFS.
2. **Channels** -- typed async message passing (kernel-internal).
3. **Shared Memory** -- shared memory regions for low-latency inter-process data exchange.

### Pipes

Pipes are unidirectional byte streams with a **reader half** and a **writer half**. Both halves implement the `Inode` trait from the VFS layer, so they integrate directly with the file descriptor table and can be manipulated through standard `read`/`write`/`close` syscalls.

**Key Types:**

| Type | Role |
|------|------|
| `PipeInner` | Shared state: circular buffer, wait queues, refcounts |
| `PipeReader` | Reader half; wraps `Arc<PipeInner>`, implements `Inode` |
| `PipeWriter` | Writer half; wraps `Arc<PipeInner>`, implements `Inode` |
| `CircularBuffer` | Fixed-size ring buffer (default 64 KiB) for pipe data |

**Creation:**

The `sys_pipe` syscall calls `pipe()` function, which allocates a shared `PipeInner` and returns reader and writer as trait objects. The syscall then opens both halves in the calling process's fd table and writes the two file descriptor numbers back to userspace.

**Read/Write Semantics:**

Both `PipeReader` and `PipeWriter` implement the `Inode` trait with async read/write futures.

*Reading:*
1. **Lock the buffer.** If there is data available, copy it out, wake one writer, and return the byte count.
2. **Buffer empty, writers exist.** `await` on `read_wq`. When woken, loop back to step 1.
3. **Buffer empty, no writers.** Return `Ok(0)` -- this signals EOF.

*Writing:*
1. **Lock the buffer.** First check if any readers still exist. If not, return `Err(IoError)` (EPIPE).
2. **Space available.** Copy data in, wake one reader, and return the byte count.
3. **Buffer full, readers exist.** `await` on `write_wq`. When woken, loop back to step 1.

**Direction Enforcement:**
- `PipeReader::write()` returns `Err(NotSupported)`.
- `PipeWriter::read()` returns `Err(NotSupported)`.

**Lifetime and Cleanup:**

Pipe halves use RAII through `Drop` implementations:

- **Dropping a `PipeReader`**: Decrements the `readers` count and calls `write_wq.wake_all()`. This unblocks any writers, which will then observe `readers == 0` and return EPIPE.
- **Dropping a `PipeWriter`**: Decrements the `writers` count and calls `read_wq.wake_all()`. This unblocks any readers, which will then observe `writers == 0` and return EOF.

### Channels

Source: [`kernel/kernel/src/ipc/channel.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/ipc/channel.rs)

**Design:** Bidirectional message channels support sending fixed-size messages (4 KiB max) between processes via dedicated syscalls. Each channel has two endpoints (sender/receiver), with a 16-message buffer backed by a `VecDeque`. Blocking is implemented via `HeapWaitQueue`-based async futures.

**Key Types:**

| Type | Role |
|------|------|
| `ChannelInner` | Shared state: message queues, wait queues |
| `ChannelEndpoint` | Sender or receiver handle; wraps `Arc<ChannelInner>` |

**Syscall Interface:**
- `sys_channel_create()` -- Creates a bidirectional channel, returns two endpoint IDs.
- `sys_channel_send(endpoint_id, msg_ptr, msg_len)` -- Sends a message (up to 4 KiB). Blocks via `HeapWaitQueue` if buffer is full.
- `sys_channel_recv(endpoint_id, msg_ptr, msg_len)` -- Receives a message. Blocks if buffer is empty.

**Async semantics:** Both send and receive operations return pinned boxed futures that yield to the async executor when the channel is full or empty, allowing other tasks to make progress.

### Shared Memory

Source: [`kernel/kernel/src/ipc/shm.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/ipc/shm.rs)

**Design:** Shared memory regions enable low-latency inter-process data exchange by mapping the same physical pages into multiple address spaces.

**Syscall Interface:**
- `sys_mem_create_shared(size)` -- Creates a shared memory region of the specified size. Returns a handle (SHM ID).
- `sys_mem_map_shared(shm_id, addr, perms)` -- Maps a shared memory region into the current process's address space at the specified virtual address with the given permissions.

**Key Features:**
- Reference-counted management ensures proper cleanup when the last reference is dropped.
- Physical page sharing allows changes in one process to be immediately visible in others without data copying.
- Permission control per mapping allows read-only or read-write access.

## Locking Discipline

The following lock ordering is enforced by convention (descending level order — acquire higher levels first):

| Level | Lock                  | Type         | Location                     |
|------:|-----------------------|--------------|------------------------------|
|    14 | `Executor.tasks`      | IrqSpinLock  | `kernel/sched/src/executor.rs` |
|    13 | `Executor.ready_queues` | IrqSpinLock | `kernel/core/src/sched.rs` |
|    10 | `TTY_LDISC`           | IrqSpinLock  | `kernel/kernel/src/tty/mod.rs` |
|    10 | `SCANCODE_BUF`        | IrqSpinLock  | `kernel/kernel/src/tty/mod.rs` |
|     4 | `PROCESS_TABLE`       | SpinLock     | `kernel/kernel/src/proc/mod.rs` |
|     4 | `fd_table`            | SpinLock (Arc) | `kernel/kernel/src/proc/mod.rs` |
|     4 | `address_space`       | SpinLock (Arc) | `kernel/kernel/src/proc/mod.rs` |
|     2 | `FUTEX_TABLE`         | SpinLock     | `kernel/kernel/src/ipc/futex.rs` |
|     2 | `PTY_SLAVES`          | SpinLock     | `kernel/kernel/src/tty/pty.rs` |
|     1 | `HEAP`                | SpinLock     | `kernel/mm/src/heap.rs`      |
|     0 | `LOGGER`              | SpinLock     | `kernel/kernel/src/log.rs` |
|     0 | `TTY_WAKER`           | IrqSpinLock  | `kernel/kernel/src/tty/mod.rs` |
|     0 | `TTY_FBCON`           | SpinLock     | `kernel/kernel/src/tty/mod.rs` |
|     0 | `FBCON`               | SpinLock     | `kernel/kernel/src/drivers/fbcon/mod.rs` |

**Key rules:**
- Never call `waker.wake()` while holding a lock — it acquires `ready_queues` (level 13). Take the waker out first, release the lock, then invoke it.
- `SpinLock::lock()` panics (with `hadron_lock_debug`) if called while any `IrqSpinLock` is held. The heap allocator uses `lock_unchecked()` to bypass this.
- `LOGGER` and `FBCON` are level 0 (no ordering check) because they sit at the bottom of the call chain.
