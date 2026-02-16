# Phase 11: IPC & Minimal Signals

## Goal

Implement async IPC primitives (pipes, channels), minimal POSIX signal handling, `sys_spawn` for process creation, and `sys_waitpid` for process reaping. This phase replaces the traditional fork/exec/CoW model with async-native primitives that integrate directly with the executor.

## No fork(), No Copy-on-Write

This kernel does not implement `fork()` or copy-on-write page fault handling. Process creation is exclusively through `sys_spawn`, which takes a path to an ELF binary and arguments, creates a new address space, and spawns an async `process_task` on the executor. This eliminates the complexity of CoW page tables and shared-page reference counting.

## Async Channels (Kernel-Internal)

For kernel-internal IPC between async tasks, two channel types are provided:

### mpsc (multi-producer, single-consumer)

```rust
pub struct Sender<T> { /* ... */ }
pub struct Receiver<T> { /* ... */ }

pub fn channel<T>() -> (Sender<T>, Receiver<T>);

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), SendError<T>>;
}

impl<T> Receiver<T> {
    pub async fn recv(&self) -> Option<T>;
}
```

### oneshot (single-use)

```rust
pub struct OneshotSender<T> { /* ... */ }
pub struct OneshotReceiver<T> { /* ... */ }

pub fn oneshot<T>() -> (OneshotSender<T>, OneshotReceiver<T>);

impl<T> OneshotSender<T> {
    pub fn send(self, value: T) -> Result<(), T>;
}

impl<T> OneshotReceiver<T> {
    pub async fn recv(self) -> Result<T, RecvError>;
}
```

Both channel types use WaitQueue internally for async wakeup.

## Pipes

Pipes are byte-oriented async channels exposed to userspace as file descriptors via the VFS. A pipe consists of a shared circular buffer with reader and writer halves, each implementing the `Inode` trait with async `read` and `write`:

```rust
pub struct Pipe {
    buffer: Mutex<CircularBuffer>,
    read_wq: WaitQueue,
    write_wq: WaitQueue,
    readers: AtomicUsize,
    writers: AtomicUsize,
}

struct CircularBuffer {
    data: Box<[u8]>,    // 64 KiB default
    read_pos: usize,
    write_pos: usize,
    count: usize,
}

impl Pipe {
    pub fn new() -> (Arc<PipeReader>, Arc<PipeWriter>) {
        let pipe = Arc::new(Pipe { /* ... */ });
        (Arc::new(PipeReader(pipe.clone())), Arc::new(PipeWriter(pipe)))
    }
}
```

`PipeReader` and `PipeWriter` implement the `Inode` trait:

- **read**: If the buffer is empty and writers still exist, the task awaits `read_wq`. When data arrives, it copies from the circular buffer and wakes `write_wq`. If all writers have been dropped, returns 0 (EOF).
- **write**: If the buffer is full, the task awaits `write_wq`. When space is available, it copies into the circular buffer and wakes `read_wq`. If all readers have been dropped, returns `EPIPE` (and delivers `SIGPIPE`).

Since pipes implement `Inode` with async read/write, they integrate naturally with the VFS and file descriptor table. A `sys_pipe` syscall creates a pipe and returns two file descriptors.

## Minimal Signals

Signal support is intentionally minimal. Only the following signals are implemented:

| Signal | Number | Default Action |
|--------|--------|----------------|
| SIGINT | 2 | Terminate (Ctrl+C) |
| SIGKILL | 9 | Terminate (cannot be caught or ignored) |
| SIGSEGV | 11 | Terminate (invalid memory access) |
| SIGPIPE | 13 | Terminate (write to broken pipe) |
| SIGTERM | 15 | Terminate |
| SIGCHLD | 17 | Ignore |
| SIGCONT | 18 | Continue (resume stopped process) |
| SIGSTOP | 19 | Stop (cannot be caught or ignored) |

`SIGINT`, `SIGSTOP`, and `SIGCONT` are essential for interactive use and job control.

There are no user-installable signal handlers in this initial implementation. Signals have only their default actions: terminate, stop, continue, or ignore.

### Signal State

```rust
pub struct SignalState {
    pending: u64,    // Bitmask of pending signals
}

impl SignalState {
    pub fn send(&mut self, signal: Signal) {
        self.pending |= 1 << (signal as u8);
    }

    pub fn dequeue(&mut self) -> Option<Signal> {
        if self.pending == 0 { return None; }
        let signum = self.pending.trailing_zeros() as u8;
        self.pending &= !(1 << signum);
        Some(Signal::from_u8(signum))
    }
}
```

### Signal Delivery

Signal checks occur at two points, before the kernel transitions back to userspace:

1. **Syscall return**: after `handle_syscall()` completes, check `process.signals.dequeue()`.
2. **Preemption return**: when `UserspaceReturn::Preempted`, check pending signals before calling `yield_now().await`. This ensures `SIGKILL` is delivered within one timer tick (~1ms) even to a tight userspace loop that makes no syscalls.

Signal actions:
- `SIGKILL` and `SIGSEGV`: terminate the process immediately. The async task returns, waking `exit_notify`.
- `SIGSTOP`: suspend the process (the async task awaits a resume `WaitQueue`).
- `SIGCONT`: wake a stopped process (signal its resume `WaitQueue`).
- `SIGPIPE`: if the current syscall is a write to a broken pipe, return `-EPIPE` instead of the normal result.
- `SIGCHLD`: ignored (no action).
- `SIGINT`, `SIGTERM`: terminate the process.

See [Preemption & Scaling](../design/preemption-and-scaling.md#signal-delivery-and-preemption-interaction) for the full design.

## sys_spawn

`sys_spawn` replaces the fork+exec pattern:

```rust
pub fn sys_spawn(
    path_ptr: UserPtr<u8>,
    argv_ptr: UserPtr<*const u8>,
    envp_ptr: UserPtr<*const u8>,
) -> Result<u32, SyscallError> {
    let path = read_user_string(path_ptr)?;
    let args = read_user_string_array(argv_ptr)?;
    let envp = read_user_string_array(envp_ptr)?;

    let elf_data = vfs::read_file(&path)?;
    let process = Process::exec(&elf_data, &args, &envp)?;

    Ok(process.pid)
}
```

This spawns a new `process_task` on the executor at `Priority::Normal`.

## sys_waitpid

`sys_waitpid` is async -- it awaits the target process's `exit_notify` WaitQueue:

```rust
pub async fn sys_waitpid(pid: i32, status: &mut i32) -> Result<i32, SyscallError> {
    let target = match pid {
        -1 => find_any_child_process()?,
        p if p > 0 => find_process_by_pid(p as u32)?,
        _ => return Err(SyscallError::InvalidArgument),
    };

    // Await process exit -- yields to executor until target exits
    target.exit_notify.wait().await;

    *status = target.exit_status();
    Ok(target.pid as i32)
}
```

## Files to Create/Modify

| File | Description |
|------|-------------|
| `hadron-kernel/src/ipc/mod.rs` | Module root |
| `hadron-kernel/src/ipc/channel.rs` | mpsc and oneshot async channels |
| `hadron-kernel/src/ipc/pipe.rs` | Pipe with Inode trait implementation |
| `hadron-kernel/src/signal/mod.rs` | Signal types, SignalState |
| `hadron-kernel/src/syscall/process.rs` | `sys_spawn`, `sys_waitpid` |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| Async channels (mpsc, oneshot) | Service | Pure Rust data structures + WaitQueue |
| Pipe buffer and Inode impl | Service | Circular buffer + async read/write |
| Signal state management | Service | Bitmask operations |
| Signal delivery (syscall return check) | Service | Runs in process_task async loop |
| `sys_spawn` | Service | Uses safe exec() and executor spawn |
| `sys_waitpid` | Service | Awaits WaitQueue |

All code in this phase is service-layer. No new unsafe frame code is needed.

## Dependencies

- **Phase 9**: Userspace processes (process_task, Process struct, exec).
- **Phase 8**: VFS (pipes as file descriptors, reading ELF binaries for spawn).

## Milestone

Spawn a child process from init, pipe stdout between processes, and deliver SIGTERM:

```
[init] Spawning /bin/echo with pipe...
[init] sys_spawn("/bin/echo") -> PID 2
[init] Reading from pipe: Hello from echo!
[init] sys_waitpid(2) -> exited with status 0
[init] Sending SIGTERM to PID 3...
[init] PID 3 terminated by signal 15
```
