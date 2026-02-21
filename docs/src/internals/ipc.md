# Inter-Process Communication

Hadron provides byte-oriented **pipes** as its current IPC primitive. Pipes are
implemented as VFS-integrated circular buffers with async read/write semantics,
allowing processes to exchange data through ordinary file descriptors.

The implementation lives in `kernel/hadron-kernel/src/ipc/`, which currently
exposes a single submodule:

- `ipc/mod.rs` -- module root
- `ipc/pipe.rs` -- pipe implementation

Future phases will add typed async channels (`mpsc`, `oneshot`) for
kernel-internal IPC between async tasks. See the
[Phase 11](../phases/11-ipc-signals.md) plan for details.

## Pipe Architecture

A pipe is a unidirectional byte stream with a **reader half** and a **writer
half**. Both halves implement the `Inode` trait from the VFS layer, so they
integrate directly with the file descriptor table and can be manipulated through
standard `read`/`write`/`close` syscalls.

### Key Types

| Type | Role |
|------|------|
| `PipeInner` | Shared state: circular buffer, wait queues, refcounts |
| `PipeReader` | Reader half; wraps `Arc<PipeInner>`, implements `Inode` |
| `PipeWriter` | Writer half; wraps `Arc<PipeInner>`, implements `Inode` |
| `CircularBuffer` | Fixed-size ring buffer for pipe data |

### Creation

The public entry point is the `pipe()` function in `ipc/pipe.rs`:

```rust
pub fn pipe() -> (Arc<dyn Inode>, Arc<dyn Inode>)
```

This allocates a shared `PipeInner` and returns the reader and writer as
trait objects. Both halves are reference-counted via `Arc`, and the caller
receives them as `Arc<dyn Inode>` -- ready to be inserted into a process's
file descriptor table.

The `sys_pipe` syscall handler (in `syscall/vfs.rs`) calls this function,
opens both halves in the calling process's fd table, and writes the two
file descriptor numbers back to userspace:

```rust
let (reader, writer) = crate::ipc::pipe::pipe();
let mut fd_table = process.fd_table.lock();
let rfd = fd_table.open(reader, OpenFlags::READ);
let wfd = fd_table.open(writer, OpenFlags::WRITE);
```

## Circular Buffer

The internal `CircularBuffer` is a fixed-size ring buffer backed by a
heap-allocated boxed slice. The default capacity is **64 KiB**
(`PIPE_BUF_SIZE`).

```rust
struct CircularBuffer {
    data: Box<[u8]>,
    read_pos: usize,
    write_pos: usize,
    count: usize,
}
```

- `read_pos` and `write_pos` track the current read/write cursors, wrapping
  around the buffer using modular arithmetic (`% capacity`).
- `count` tracks how many bytes are currently stored, which determines both
  emptiness (`count == 0`) and fullness (`count == capacity`).
- `read()` copies up to `min(buf.len(), count)` bytes out and advances
  `read_pos`.
- `write()` copies up to `min(buf.len(), capacity - count)` bytes in and
  advances `write_pos`.

Both operations are partial: they transfer as many bytes as possible in a
single call and return the number of bytes actually transferred.

## Shared State

The `PipeInner` struct holds all shared state between the reader and writer:

```rust
struct PipeInner {
    buffer: SpinLock<CircularBuffer>,
    read_wq: HeapWaitQueue,
    write_wq: HeapWaitQueue,
    readers: AtomicUsize,
    writers: AtomicUsize,
}
```

- **`buffer`** -- The circular buffer, protected by a `SpinLock`. The lock is
  held only for the duration of a `read` or `write` on the buffer (short
  critical section).
- **`read_wq`** / **`write_wq`** -- Async wait queues (`HeapWaitQueue`) that
  allow tasks to yield the CPU while waiting for data or space. These use an
  `IrqSpinLock<VecDeque<Waker>>` internally, providing FIFO wakeup order.
- **`readers`** / **`writers`** -- Atomic reference counts tracking how many
  reader and writer handles exist. These determine EOF and broken-pipe
  conditions.

## Read/Write Semantics

Both `PipeReader` and `PipeWriter` implement the `Inode` trait. The key methods
are `read` and `write`, which return pinned boxed futures for async operation.

### Reading (`PipeReader::read`)

The read loop operates as follows:

1. **Lock the buffer.** If there is data available, copy it out, wake one
   writer (via `write_wq.wake_one()`), and return the byte count.
2. **Buffer empty, writers exist.** Drop the lock and `await` on `read_wq`.
   When woken, loop back to step 1.
3. **Buffer empty, no writers.** Return `Ok(0)` -- this signals EOF to the
   caller.

Reads are non-blocking when data is available and yield to the async executor
when the buffer is empty. The `offset` parameter from the `Inode` trait is
ignored since pipes are sequential streams.

### Writing (`PipeWriter::write`)

The write loop operates as follows:

1. **Lock the buffer.** First check if any readers still exist. If not, return
   `Err(FsError::IoError)` (the EPIPE condition).
2. **Space available.** Copy data in, wake one reader (via
   `read_wq.wake_one()`), and return the byte count.
3. **Buffer full, readers exist.** Drop the lock and `await` on `write_wq`.
   When woken, loop back to step 1.

Like reads, writes are partial -- they transfer whatever fits in the available
space and return immediately. The `offset` parameter is ignored.

### Direction Enforcement

Each half rejects operations in the wrong direction:
- `PipeReader::write()` returns `Err(FsError::NotSupported)`.
- `PipeWriter::read()` returns `Err(FsError::NotSupported)`.

This is also reflected in the `Permissions` reported by each half:
- `PipeReader` reports `Permissions::read_only()`.
- `PipeWriter` reports write-only permissions (`read: false, write: true,
  execute: false`).

Both halves report `InodeType::CharDevice` and reject all directory operations
(`lookup`, `readdir`, `create`, `unlink`) with `FsError::NotADirectory`.

## Lifetime and Cleanup

Pipe halves use RAII through `Drop` implementations:

- **Dropping a `PipeReader`**: Decrements the `readers` count and calls
  `write_wq.wake_all()`. This unblocks any writers, which will then observe
  `readers == 0` and return EPIPE.
- **Dropping a `PipeWriter`**: Decrements the `writers` count and calls
  `read_wq.wake_all()`. This unblocks any readers, which will then observe
  `writers == 0` and return EOF (0 bytes).

The `wake_all()` calls are important: all blocked tasks must be woken so they
can re-check the termination condition, not just one.

## VFS Integration

Because `PipeReader` and `PipeWriter` implement `Inode`, pipes participate in
the VFS layer without any special cases:

- The `sys_pipe` syscall creates both halves and inserts them into the
  process's file descriptor table using the standard `fd_table.open()` path.
- Subsequent `sys_read` and `sys_write` syscalls dispatch to the pipe's
  `Inode::read` and `Inode::write` methods through the normal VFS codepath.
- Closing a file descriptor drops the `Arc<dyn Inode>`, which may trigger the
  `Drop` implementation if it was the last reference.

This design means pipes require no special-case handling in the syscall
layer -- they are just inodes with async read/write behavior.

## Async Integration

Pipe I/O integrates with Hadron's cooperative async executor. When a read or
write cannot proceed (empty buffer or full buffer), the future yields by
awaiting a `HeapWaitQueue`. The executor is then free to schedule other tasks.

The `HeapWaitQueue` stores wakers in a `VecDeque` protected by an
`IrqSpinLock`, ensuring interrupt-safe FIFO wakeup. When the counterpart
operation completes (a write fills data, or a read frees space), it calls
`wake_one()` to re-schedule exactly one waiting task.

For synchronous contexts (e.g., early boot or kernel code that cannot yield),
the VFS layer provides `try_poll_immediate()`, which attempts a single poll of
the future. If the pipe operation would block, it returns `None` rather than
yielding.

## Current Limitations

- **No atomic write guarantee**: Writes smaller than `PIPE_BUF_SIZE` are not
  guaranteed to be atomic. A partial write can occur if the buffer has less
  free space than the write size.
- **Single buffer size**: All pipes use the same 64 KiB buffer. There is no
  mechanism to configure per-pipe buffer sizes.
- **No typed channels yet**: The `mpsc` and `oneshot` async channels planned in
  Phase 11 are not yet implemented. Only byte-oriented pipes are available.
- **No SIGPIPE delivery**: When a write encounters a broken pipe, it returns
  `FsError::IoError` but does not deliver a `SIGPIPE` signal to the writing
  process (signals are not yet implemented).
