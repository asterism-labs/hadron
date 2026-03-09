# Syscall Interface

Hadron's syscall interface is defined in a single source file, `kernel/syscall/src/lib.rs`, using
the `define_syscalls!` proc macro from `kernel/syscall-macros`. The macro generates:

- Syscall number constants (`SYS_*`) for the dispatch table.
- Typed `#[repr(C)]` data structures shared between the kernel and userspace.
- Named constants for query topics, clock IDs, open flags, and signal numbers.
- A `Syscall` enum and a `SyscallGroup` enum with introspection methods (name, number, group).
- (kernel feature) A `SyscallHandler` trait and a `dispatch(nr, args)` function.
- (userspace feature) Raw `syscallN` assembly stubs and typed wrapper functions in the userspace
  library.

This means the syscall numbers, argument layouts, and return conventions are defined exactly once
and cannot drift between the kernel handler and the userspace stubs.

---

## Calling Convention

Syscalls use the `syscall` instruction on x86_64. Registers follow the Linux x86_64 ABI convention
(chosen for compatibility with the Rust standard library's inline assembly and libc expectations):

| Register | Role |
|----------|------|
| `rax`    | Syscall number (input) / return value (output) |
| `rdi`    | Argument 0 |
| `rsi`    | Argument 1 |
| `rdx`    | Argument 2 |
| `r10`    | Argument 3 (not `rcx`, which is clobbered by `syscall`) |
| `r8`     | Argument 4 |
| `r9`     | Argument 5 |
| `rcx`    | Saved RIP (clobbered by CPU) |
| `r11`    | Saved RFLAGS (clobbered by CPU) |

Return values are in `rax`. Errors are returned as negated errno values (e.g., `-EINVAL` = `-22`).
Success values are non-negative. This matches the Linux convention used by `musl` and the Rust
standard library.

---

## Error Codes

The macro defines standard POSIX errno values. Key codes used throughout the interface:

| Constant    | Value | Meaning |
|-------------|-------|---------|
| `ENOENT`    | 2     | No such file or directory |
| `EIO`       | 5     | I/O error |
| `EBADF`     | 9     | Bad file descriptor |
| `ENOMEM`    | 12    | Out of memory |
| `EACCES`    | 13    | Permission denied |
| `EFAULT`    | 14    | Bad address (invalid user pointer) |
| `EEXIST`    | 17    | File exists |
| `EINVAL`    | 22    | Invalid argument |
| `EAGAIN`    | 11    | Resource temporarily unavailable |
| `ENOSYS`    | 38    | Function not implemented |
| `EMSGSIZE`  | 90    | Message too large for channel |
| `ECONNREFUSED` | 111 | Connection refused |

---

## Syscall Groups

Syscalls are organized into named groups. Each group occupies a contiguous range of syscall numbers.
The `define_syscalls!` macro encodes the range as `group name(START..END)`.

### task (0x00–0x0F): Task Management

Manages the lifecycle and state of processes and threads.

| Syscall | Arguments | Description |
|---------|-----------|-------------|
| `task_exit` | `status` | Terminate the calling task. Does not return. |
| `task_spawn` | `info_ptr`, `info_len` | Spawn a child process from an ELF binary. `info_ptr` points to a `SpawnInfo` struct containing the path, argv, envp, fd map, and CWD. |
| `task_wait` | `pid`, `status_ptr`, `flags` | Wait for a child to exit. `flags` may be `WNOHANG` or `WUNTRACED`. Returns the child PID. |
| `task_kill` | `pid`, `signum` | Send a signal to a process. |
| `task_clone` | `flags`, `stack_ptr`, `tls_ptr` | Clone the current task (create a thread). `flags` combines `CLONE_VM`, `CLONE_FILES`, `CLONE_SIGHAND`. Returns TID in parent, 0 in child. |
| `task_info` | — | Return the calling task's PID. |
| `task_sigaction` | `signum`, `handler`, `flags`, `old_out` | Register a signal handler. `handler` is `SIG_DFL`, `SIG_IGN`, or a function pointer. |
| `task_sigreturn` | — | Restore pre-signal context from the signal frame on the user stack. Called from the signal trampoline. |
| `task_setpgid` | `pid`, `pgid` | Set the process group ID. |
| `task_getpgid` | `pid` | Get the process group ID. |
| `task_getppid` | — | Get the parent process ID. |
| `task_getcwd` | `buf_ptr`, `buf_len` | Copy the current working directory into a user buffer. |
| `task_chdir` | `path_ptr`, `path_len` | Change the current working directory. |
| `task_setsid` | — | Create a new session; the caller becomes session and process group leader. |
| `task_sigprocmask` | `how`, `set`, `oldset_out` | Set the signal mask. `how` is `SIG_BLOCK`, `SIG_UNBLOCK`, or `SIG_SETMASK`. |
| `task_execve` | `info_ptr`, `info_len` | Replace the current process image. Reuses the same PID, fd table, and CWD. Signal handlers reset to `SIG_DFL`. |

The `SpawnInfo` struct is `#[repr(C)]` and versioned by `info_len`. The kernel validates
`info_len >= size_of::<SpawnInfo>()` before dereferencing any field.

### handle (0x10–0x1F): Handle Operations

Manages the per-process file descriptor table. In Hadron, "file descriptors" and "handles" are
the same concept: an integer index into the process's handle table.

| Syscall | Arguments | Description |
|---------|-----------|-------------|
| `handle_close` | `fd` | Release a handle, decrementing the kernel object's reference count. |
| `handle_dup` | `old_fd`, `new_fd` | Duplicate `old_fd` to `new_fd` (dup2 semantics). Closes `new_fd` if open. |
| `handle_dup_lowest` | `old_fd` | Duplicate to the lowest available fd number. Returns the new fd. |
| `handle_pipe` | `fds_ptr` | Create a unidirectional pipe. Writes `[read_fd, write_fd]` to `fds_ptr`. |
| `handle_pipe2` | `fds_ptr`, `flags` | Pipe with flags: `PIPE_CLOEXEC`, `PIPE_NONBLOCK`. |
| `handle_ioctl` | `fd`, `cmd`, `arg_ptr` | Device-specific ioctl. `cmd` selects the operation; `arg_ptr` points to a typed argument struct. |
| `handle_fcntl` | `fd`, `cmd`, `arg` | File control: `F_DUPFD`, `F_GETFD`, `F_SETFD`, `F_GETFL`, `F_SETFL`, `F_DUPFD_CLOEXEC`. |
| `handle_tcsetpgrp` | `fd`, `pgid` | Set the foreground process group of a terminal. |
| `handle_tcgetpgrp` | `fd` | Get the foreground process group of a terminal. |

The `handle_ioctl` syscall dispatches device-specific commands. Framebuffer ioctls (`FBIOGET_INFO`,
`FBIOBLANK`, `FBIODIRTY`) and terminal ioctls (`TCGETS`, `TCSETS`, `TIOCGWINSZ`, `TIOCSWINSZ`,
`TIOCGPTN`, `TIOCSPTLCK`) are the primary consumers.

### channel (0x20–0x2F): Channel IPC

Channels are the primary IPC mechanism. Each channel is a bidirectional, message-oriented
connection between two endpoints. Messages are discrete packets up to 4096 bytes. A channel
endpoint is represented as a file descriptor.

| Syscall | Arguments | Description |
|---------|-----------|-------------|
| `channel_create` | `fds_ptr` | Create a channel pair. Writes `[fd_a, fd_b]` to `fds_ptr`. Each endpoint can send and receive. |
| `channel_send` | `fd`, `buf_ptr`, `buf_len` | Send a message (up to 4096 bytes). Blocks if the send queue is full. |
| `channel_recv` | `fd`, `buf_ptr`, `buf_len` | Receive the next queued message. Blocks if the queue is empty. Returns message length. |
| `channel_accept` | `listener_fd` | Accept a pending connection from a `ServiceListener` inode. Blocks if no connections are pending. Returns a new channel fd. |
| `channel_send_fd` | `fd`, `fd_to_send`, `buf_ptr`, `buf_len` | Send a message with an attached file descriptor. The receiver retrieves the fd via `channel_recv_fd`. |
| `channel_recv_fd` | `fd`, `buf_ptr`, `buf_len`, `fd_out_ptr` | Receive a message, optionally with an attached fd. Writes the received fd to `fd_out_ptr` (`usize::MAX` if none). |

Channels support fd passing (`SCM_RIGHTS` equivalent) via `channel_send_fd` / `channel_recv_fd`.
This is how `devmgr` distributes capability handles (MMIO VMOs, Interrupt objects, Bti handles)
to driver processes without going through the kernel's root resource.

### vnode (0x30–0x3F): Filesystem / VFS

VFS operations on paths and open file descriptors. The kernel's VFS router dispatches path-based
operations to the appropriate filesystem server channel. See [VFS Routing Layer](vfs-routing.md).

| Syscall | Arguments | Description |
|---------|-----------|-------------|
| `vnode_open` | `path_ptr`, `path_len`, `flags` | Open a file or directory by path. `flags` combines `OPEN_READ`, `OPEN_WRITE`, `OPEN_CREATE`, `OPEN_TRUNCATE`, `OPEN_CLOEXEC`, `OPEN_DIRECTORY`, `OPEN_NONBLOCK`, etc. Returns an fd. |
| `vnode_read` | `fd`, `buf_ptr`, `buf_len` | Read bytes from an open file. Returns bytes read or negated errno. |
| `vnode_write` | `fd`, `buf_ptr`, `buf_len` | Write bytes to an open file. Returns bytes written or negated errno. |
| `vnode_stat` | `fd`, `buf_ptr`, `buf_len` | Write a `StatInfo` struct describing the file. Includes inode type, size, and permissions. |
| `vnode_readdir` | `fd`, `buf_ptr`, `buf_len` | Read directory entries as a `DirEntryInfo` array. Returns total bytes written. |
| `vnode_unlink` | `path_ptr`, `path_len` | Remove a file or empty directory. |
| `vnode_seek` | `fd`, `offset`, `whence` | Seek to a position. `whence` is `SEEK_SET`, `SEEK_CUR`, or `SEEK_END`. Returns new offset. |
| `vnode_mkdir` | `path_ptr`, `path_len`, `perm` | Create a directory. |
| `vnode_rename` | `old_ptr`, `old_len`, `new_ptr`, `new_len` | Rename or move a file or directory. |
| `vnode_symlink` | `target_ptr`, `target_len`, `link_ptr`, `link_len` | Create a symbolic link. |
| `vnode_link` | `target_ptr`, `target_len`, `link_ptr`, `link_len` | Create a hard link. |
| `vnode_readlink` | `path_ptr`, `path_len`, `buf_ptr`, `buf_len` | Read a symlink's target. |
| `vnode_truncate` | `fd`, `len` | Truncate a file to `len` bytes. |
| `vnode_fstatat` | `dirfd`, `path_ptr`, `path_len`, `buf`, `flags` | Stat relative to a directory fd. `dirfd` may be `AT_FDCWD`. `flags` may include `AT_SYMLINK_NOFOLLOW`. |

### memory (0x40–0x4F): Memory Management

Anonymous and shared memory mapping. The VMAR tree manages the virtual address space per process.

| Syscall | Arguments | Description |
|---------|-----------|-------------|
| `mem_map` | `addr_hint`, `length`, `prot`, `flags`, `fd` | Map memory. `prot` combines `PROT_READ`, `PROT_WRITE`, `PROT_EXEC`. `flags` is `MAP_ANONYMOUS` or `MAP_SHARED`. Returns mapped virtual address. |
| `mem_unmap` | `addr`, `length` | Unmap a previously mapped region. `addr` and `length` must match the original mapping. |
| `mem_brk` | `addr` | Adjust the program break (sbrk-compatible heap extension). Returns current break if `addr` is 0. |
| `mem_create_shared` | `size` | Allocate a shared memory object. Returns an fd referring to the zero-filled anonymous memory. |
| `mem_map_shared` | `fd`, `size`, `prot` | Map a shared memory object into the address space. Returns the virtual address. |
| `mem_protect` | `addr`, `length`, `prot` | Change protection flags on a mapped region. Returns `EINVAL` if not page-aligned. |

### event (0x50–0x5F): Events and Time

Asynchronous notification and time-related syscalls.

| Syscall | Arguments | Description |
|---------|-----------|-------------|
| `event_create` | — | Reserved for future use. |
| `event_signal` | `handle` | Reserved for future use. |
| `event_wait` | `handle` | Reserved for future use. |
| `event_wait_many` | `fds_ptr`, `nfds`, `timeout_ms` | Poll multiple fds for readiness. `fds_ptr` is a `PollFd` array. `timeout_ms` is `usize::MAX` for infinite. Returns number of ready fds. |
| `clock_gettime` | `clock_id`, `tp` | Write a `Timespec` to `tp`. `CLOCK_MONOTONIC` is nanoseconds since boot; `CLOCK_REALTIME` is wall-clock Unix time. |
| `clock_nanosleep` | `clock_id`, `flags`, `req_ptr`, `rem_ptr` | Sleep for the duration in `Timespec` at `req_ptr`. Remaining time written to `rem_ptr` if interrupted. |
| `futex` | `addr`, `op`, `val`, `timeout_ms` | Fast userspace mutex primitive. `FUTEX_WAIT` sleeps if `*addr == val`; `FUTEX_WAKE` wakes up to `val` waiters. |

`event_wait_many` is the primary readiness notification mechanism, implementing `poll(2)` semantics.
The `PollFd` struct contains `fd`, `events` (requested), and `revents` (returned). Events include
`POLLIN`, `POLLOUT`, `POLLERR`, `POLLHUP`, `POLLNVAL`.

### net (0x60–0x6F): AF_UNIX Sockets

Hadron currently implements only `AF_UNIX` `SOCK_STREAM` sockets. Network sockets (IPv4/IPv6) are
handled by a userspace networking stack communicating over channels with the kernel.

| Syscall | Arguments | Description |
|---------|-----------|-------------|
| `socket` | `domain`, `type_`, `protocol` | Create a socket. `domain` must be `AF_UNIX` (1). `type_` must be `SOCK_STREAM` (1). |
| `bind` | `fd`, `addr_ptr`, `addr_len` | Bind to a filesystem path (`struct sockaddr_un`). |
| `listen` | `fd`, `backlog` | Mark the socket as listening. `backlog` limits pending connections. |
| `accept` | `fd`, `addr_ptr`, `addr_len_ptr` | Accept a connection. Blocks until one is available. Returns a new connected fd. |
| `connect` | `fd`, `addr_ptr`, `addr_len` | Connect to a listening socket by path. |
| `sendmsg` | `fd`, `msg_ptr`, `flags` | Send a message via a POSIX `struct msghdr`. |
| `recvmsg` | `fd`, `msg_ptr`, `flags` | Receive a message into a POSIX `struct msghdr`. |
| `shutdown` | `fd`, `how` | Shut down part of the connection. `how` is `SHUT_RD`, `SHUT_WR`, or `SHUT_RDWR`. |

### system (0xF0–0xFF): System Services

| Syscall | Arguments | Description |
|---------|-----------|-------------|
| `query` | `topic`, `sub_id`, `out_buf`, `out_len` | Query typed system information. `topic` selects the kind; the response is a typed `#[repr(C)]` struct. |
| `debug_log` | `buf`, `len` | Write a string to the kernel serial console. Available in all builds (not gated on a debug feature flag). |

Query topics:

| Constant | Value | Response type |
|----------|-------|---------------|
| `QUERY_MEMORY` | 0 | `MemoryInfo` (total, free, used bytes) |
| `QUERY_UPTIME` | 1 | `UptimeInfo` (nanoseconds since boot) |
| `QUERY_KERNEL_VERSION` | 2 | `KernelVersionInfo` (major, minor, patch, name) |
| `QUERY_PROCESSES` | 3 | `ProcessInfo` (process count) |
| `QUERY_VMAPS` | 4 | Array of `VmapEntry` (virtual memory map of the calling process) |
| `QUERY_CPU_INFO` | 5 | `CpuInfo` (core count, feature flags, model string) |

---

## Shared Types

All types are `#[repr(C)]` and `Copy`. Pointer fields carry the same size on any supported architecture.

| Type | Description |
|------|-------------|
| `Timespec` | `{ tv_sec: u64, tv_nsec: u64 }` — boot-relative monotonic time. Both fields are `u64` because negative timestamps are impossible. |
| `MemoryInfo` | Physical memory statistics. |
| `UptimeInfo` | Nanoseconds since boot. |
| `KernelVersionInfo` | Version tuple + 32-byte name string. |
| `CpuInfo` | Core count, CPU feature bitmask, 48-byte model string. |
| `StatInfo` | Inode type, size, permissions, device number. |
| `VmapEntry` | One virtual mapping: start, end, protection flags, 16-byte name. |
| `SpawnInfo` | Spawn descriptor: path, argv, envp, fd map, CWD — all as `(ptr, len)` pairs. |
| `SpawnArg` | One `(ptr, len)` string argument within a `SpawnInfo`. |
| `FdMapEntry` | `{ child_fd: u32, parent_fd: u32 }` — fd inheritance during spawn. |
| `PollFd` | `{ fd: u32, events: u16, revents: u16 }` — readiness notification descriptor. |
| `DirEntryInfo` | Inode type + name (up to 60 bytes) for directory entries. |
| `FbInfo` | Framebuffer geometry: width, height, pitch, bpp, pixel format. |
| `FbDirtyRect` | Dirty rectangle `(x, y, width, height)` for `FBIODIRTY` ioctl. |
| `Termios` | Terminal I/O settings: `iflag`, `oflag`, `cflag`, `lflag`, `cc[32]`. |
| `Winsize` | Terminal size: rows, columns, pixel dimensions. |
| `MouseEventPacket` | `{ dx: i16, dy: i16, buttons: u8, _pad: [u8; 3] }`. |

---

## The `define_syscalls!` Macro

The macro is the single source of truth for the kernel/userspace ABI. Adding a new syscall requires
only editing `kernel/syscall/src/lib.rs`. The macro:

1. Assigns the next available number within the group range.
2. Generates a `SYS_<name>` constant.
3. Adds an arm to the kernel's `dispatch()` match expression, calling `SyscallHandler::<name>`.
4. Generates a typed userspace wrapper in `hadron-libc` that marshals arguments and invokes the
   raw `syscallN` stub.

Syscalls marked `#[reserved]` are allocated a number but have no dispatch arm — attempting to call
them returns `ENOSYS`. This allows numbers to be reserved for future use without breaking the
existing dispatch table layout.
