# Syscall Reference

Hadron syscalls use the `syscall` instruction on x86_64. Arguments are passed in registers `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` (in order). The syscall number is in `rax`. The return value is in `rax`; negative values are negated errno codes.

Syscall numbers are grouped in 16-entry blocks. The group base is the high nibble; the entry within the group is the low nibble.

## Error Codes

All syscalls return a non-negative value on success. On failure they return a negated errno:

| Constant | Value | Meaning |
|----------|-------|---------|
| `ENOENT` | -2 | No such file or directory |
| `ESRCH` | -3 | No such process |
| `EINTR` | -4 | Interrupted system call |
| `EIO` | -5 | I/O error |
| `EBADF` | -9 | Bad file descriptor |
| `ECHILD` | -10 | No child processes |
| `EAGAIN` | -11 | Resource temporarily unavailable |
| `ENOMEM` | -12 | Out of memory |
| `EACCES` | -13 | Permission denied |
| `EFAULT` | -14 | Bad address (user pointer validation failed) |
| `EEXIST` | -17 | File exists |
| `ENOTDIR` | -20 | Not a directory |
| `EISDIR` | -21 | Is a directory |
| `EINVAL` | -22 | Invalid argument |
| `ESPIPE` | -29 | Illegal seek |
| `EPIPE` | -32 | Broken pipe |
| `ENAMETOOLONG` | -36 | File name too long |
| `ENOSYS` | -38 | Function not implemented |
| `ELOOP` | -40 | Too many levels of symbolic links |
| `EMSGSIZE` | -90 | Message too long |
| `ENOTSOCK` | -88 | Socket operation on non-socket |
| `EADDRINUSE` | -98 | Address already in use |
| `EISCONN` | -106 | Transport endpoint is already connected |
| `ENOTCONN` | -107 | Transport endpoint is not connected |
| `ECONNREFUSED` | -111 | Connection refused |

---

## Group: task (0x00–0x0F)

Task management: process and thread lifecycle, signals, process groups.

| Number | Name | Parameters | Returns | Description |
|--------|------|-----------|---------|-------------|
| `0x00` | `task_exit` | `status: usize` | never | Terminate the calling task with the given exit status |
| `0x01` | `task_spawn` | `info_ptr: usize`, `info_len: usize` | child PID or negated errno | Spawn a new process from an ELF binary. `info_ptr` points to a `SpawnInfo` struct |
| `0x02` | `task_wait` | `pid: usize`, `status_ptr: usize`, `flags: usize` | child PID or negated errno | Wait for a child to exit; write its exit status to `*status_ptr`. `flags`: `WNOHANG`, `WUNTRACED` |
| `0x03` | `task_kill` | `pid: usize`, `signum: usize` | 0 or negated errno | Send signal `signum` to process `pid` |
| `0x04` | `task_clone` | `flags: usize`, `stack_ptr: usize`, `tls_ptr: usize` | TID (parent) or 0 (child) | Create a new thread. `flags`: `CLONE_VM`, `CLONE_FILES`, `CLONE_SIGHAND`, `CLONE_SETTLS` |
| `0x05` | `task_info` | — | task ID | Return the current process's task ID (PID) |
| `0x06` | `task_sigaction` | `signum: usize`, `handler: usize`, `flags: usize`, `old_handler_out: usize` | 0 or negated errno | Register a signal handler. `handler` is `SIG_DFL` (0), `SIG_IGN` (1), or a function pointer. `flags`: `SA_RESTART`, `SA_RESETHAND` |
| `0x07` | `task_sigreturn` | — | (restores user context) | Called by the signal trampoline to restore state after a signal handler returns |
| `0x08` | `task_setpgid` | `pid: usize`, `pgid: usize` | 0 or negated errno | Set the process group ID of `pid`. `pid=0` means calling process; `pgid=0` means use `pid` |
| `0x09` | `task_getpgid` | `pid: usize` | PGID or negated errno | Get the process group ID of `pid` (`0` = calling process) |
| `0x0A` | `task_getppid` | — | PPID | Return the calling process's parent PID (0 if no parent) |
| `0x0B` | `task_getcwd` | `buf_ptr: usize`, `buf_len: usize` | length or negated errno | Copy the current working directory path into `buf` |
| `0x0C` | `task_chdir` | `path_ptr: usize`, `path_len: usize` | 0 or negated errno | Change the current working directory |
| `0x0D` | `task_setsid` | — | session ID or negated errno | Create a new session; caller becomes session leader |
| `0x0E` | `task_sigprocmask` | `how: usize`, `set: usize`, `oldset_out: usize` | 0 or negated errno | Modify the calling process's signal mask. `how`: `SIG_BLOCK`, `SIG_UNBLOCK`, `SIG_SETMASK` |
| `0x0F` | `task_execve` | `info_ptr: usize`, `info_len: usize` | negated errno (only on failure) | Replace the calling process image with a new program |

### SpawnInfo Structure

`task_spawn` and `task_execve` take a pointer to a `SpawnInfo` struct:

```c
struct SpawnInfo {
    uintptr_t path_ptr;     // pointer to UTF-8 path string
    uintptr_t path_len;     // length of path string
    uintptr_t argv_ptr;     // pointer to SpawnArg[] for argv
    uintptr_t argv_count;   // number of argv entries
    uintptr_t envp_ptr;     // pointer to SpawnArg[] for envp (KEY=value strings)
    uintptr_t envp_count;   // number of envp entries
    uintptr_t fd_map_ptr;   // pointer to FdMapEntry[] (null = inherit fds 0/1/2)
    uintptr_t fd_map_count; // number of fd map entries
    uintptr_t cwd_ptr;      // pointer to CWD path (null = inherit from parent)
    uintptr_t cwd_len;      // length of CWD path
};
```

---

## Group: handle (0x10–0x1F)

File descriptor operations.

| Number | Name | Parameters | Returns | Description |
|--------|------|-----------|---------|-------------|
| `0x10` | `handle_close` | `handle: usize` | 0 or negated errno | Close a file descriptor |
| `0x11` | `handle_dup` | `old_fd: usize`, `new_fd: usize` | `new_fd` or negated errno | Duplicate `old_fd` to `new_fd` (dup2 semantics) |
| `0x12` | `handle_dup_lowest` | `old_fd: usize` | new fd or negated errno | Duplicate `old_fd` to the lowest available fd number |
| `0x13` | `handle_pipe` | `fds_ptr: usize` | 0 or negated errno | Create a pipe; writes `[read_fd, write_fd]` to `*fds_ptr` |
| `0x14` | `handle_tcsetpgrp` | `fd: usize`, `pgid: usize` | 0 or negated errno | Set the foreground process group of the terminal associated with `fd` |
| `0x15` | `handle_tcgetpgrp` | `fd: usize` | PGID or negated errno | Get the foreground process group of the terminal associated with `fd` |
| `0x16` | `handle_ioctl` | `fd: usize`, `cmd: usize`, `arg_ptr: usize` | 0 or negated errno | Perform a device-specific ioctl. `cmd` is a device-defined command number |
| `0x17` | `handle_fcntl` | `fd: usize`, `cmd: usize`, `arg: usize` | cmd-dependent or negated errno | Perform an fcntl operation: `F_DUPFD`, `F_GETFD`, `F_SETFD`, `F_GETFL`, `F_SETFL`, `F_DUPFD_CLOEXEC` |
| `0x18` | `handle_pipe2` | `fds_ptr: usize`, `flags: usize` | 0 or negated errno | Create a pipe with flags. `flags`: `PIPE_CLOEXEC`, `PIPE_NONBLOCK` |

---

## Group: channel (0x20–0x2F)

Bidirectional message-passing IPC.

| Number | Name | Parameters | Returns | Description |
|--------|------|-----------|---------|-------------|
| `0x20` | `channel_create` | `fds_ptr: usize` | 0 or negated errno | Create a channel pair; writes `[fd_a, fd_b]` to `*fds_ptr` |
| `0x21` | `channel_send` | `handle: usize`, `buf_ptr: usize`, `buf_len: usize` | 0 or negated errno | Send a message (up to 4096 bytes). Blocks if the send queue is full |
| `0x22` | `channel_recv` | `handle: usize`, `buf_ptr: usize`, `buf_len: usize` | bytes received or negated errno | Receive a message. Blocks if the queue is empty. Message is truncated if buffer is too small |
| `0x23` | `channel_accept` | `listener_fd: usize` | new channel fd or negated errno | Accept a pending connection from a service listener inode. Blocks if no connections are pending |
| `0x24` | `channel_send_fd` | `handle: usize`, `fd_to_send: usize`, `buf_ptr: usize`, `buf_len: usize` | 0 or negated errno | Send a message with one attached file descriptor |
| `0x25` | `channel_recv_fd` | `handle: usize`, `buf_ptr: usize`, `buf_len: usize`, `fd_out_ptr: usize` | bytes received or negated errno | Receive a message and its attached fd. `*fd_out_ptr` is set to `usize::MAX` if no fd was attached |

---

## Group: vnode (0x30–0x3F)

Filesystem and vnode operations. All path arguments are UTF-8 byte slices (not NUL-terminated).

| Number | Name | Parameters | Returns | Description |
|--------|------|-----------|---------|-------------|
| `0x30` | `vnode_open` | `path_ptr: usize`, `path_len: usize`, `flags: usize` | fd or negated errno | Open a vnode by path. `flags`: `OPEN_READ`, `OPEN_WRITE`, `OPEN_CREATE`, `OPEN_TRUNCATE`, `OPEN_APPEND`, `OPEN_CLOEXEC`, `OPEN_NONBLOCK`, `OPEN_DIRECTORY`, `OPEN_EXCL`, `OPEN_NOFOLLOW` |
| `0x31` | `vnode_read` | `fd: usize`, `buf_ptr: usize`, `buf_len: usize` | bytes read or negated errno | Read from a vnode at the current offset |
| `0x32` | `vnode_write` | `fd: usize`, `buf_ptr: usize`, `buf_len: usize` | bytes written or negated errno | Write to a vnode at the current offset |
| `0x33` | `vnode_stat` | `fd: usize`, `buf_ptr: usize`, `buf_len: usize` | 0 or negated errno | Write a `StatInfo` struct to `buf` |
| `0x34` | `vnode_readdir` | `fd: usize`, `buf_ptr: usize`, `buf_len: usize` | bytes written or negated errno | Read directory entries as an array of `DirEntryInfo` structs |
| `0x35` | `vnode_unlink` | `path_ptr: usize`, `path_len: usize` | 0 or negated errno | Unlink a file or empty directory |
| `0x36` | `vnode_seek` | `fd: usize`, `offset: usize`, `whence: usize` | new offset or negated errno | Seek to a position. `whence`: `SEEK_SET`, `SEEK_CUR`, `SEEK_END` |
| `0x37` | `vnode_mkdir` | `path_ptr: usize`, `path_len: usize`, `permissions: usize` | 0 or negated errno | Create a directory |
| `0x38` | `vnode_rename` | `old_ptr: usize`, `old_len: usize`, `new_ptr: usize`, `new_len: usize` | 0 or negated errno | Rename (move) a file or directory |
| `0x39` | `vnode_symlink` | `target_ptr: usize`, `target_len: usize`, `link_ptr: usize`, `link_len: usize` | 0 or negated errno | Create a symbolic link |
| `0x3A` | `vnode_link` | `target_ptr: usize`, `target_len: usize`, `link_ptr: usize`, `link_len: usize` | 0 or negated errno | Create a hard link |
| `0x3B` | `vnode_readlink` | `path_ptr: usize`, `path_len: usize`, `buf_ptr: usize`, `buf_len: usize` | length or negated errno | Read the target of a symbolic link |
| `0x3C` | `vnode_truncate` | `fd: usize`, `len: usize` | 0 or negated errno | Truncate a file to `len` bytes |
| `0x3D` | `vnode_fstatat` | `dirfd: usize`, `path_ptr: usize`, `path_len: usize`, `buf: usize`, `flags: usize` | 0 or negated errno | Stat a path relative to a directory fd. `dirfd` may be `AT_FDCWD`. `flags`: `AT_SYMLINK_NOFOLLOW` |

### StatInfo Structure

```c
struct StatInfo {
    uint8_t  inode_type;   // 0=file, 1=dir, 2=chardev, 3=symlink, 4=blockdev, 5=socket
    uint8_t  _pad[7];
    uint64_t size;         // file size in bytes (0 for directories and devices)
    uint32_t permissions;  // bit 0=read, bit 1=write, bit 2=exec
    uint32_t _pad2;
    uint64_t rdev;         // device number (makedev encoding) for char/block devices
};
```

---

## Group: memory (0x40–0x4F)

Virtual memory management.

| Number | Name | Parameters | Returns | Description |
|--------|------|-----------|---------|-------------|
| `0x40` | `mem_map` | `addr_hint: usize`, `length: usize`, `prot: usize`, `flags: usize`, `fd: usize` | virtual address or negated errno | Map memory. `prot`: `PROT_READ`, `PROT_WRITE`, `PROT_EXEC`. `flags`: `MAP_ANONYMOUS`, `MAP_SHARED`. `addr_hint` is ignored; kernel chooses address |
| `0x41` | `mem_unmap` | `addr: usize`, `length: usize` | 0 or negated errno | Unmap a previously-mapped region |
| `0x42` | `mem_brk` | `addr: usize` | new break address or negated errno | Adjust the program break. `addr=0` returns current break |
| `0x43` | `mem_create_shared` | `size: usize` | fd or negated errno | Create a shared memory object (VMO) of `size` bytes |
| `0x44` | `mem_map_shared` | `fd: usize`, `size: usize`, `prot: usize` | virtual address or negated errno | Map a shared memory fd into the address space |
| `0x45` | `mem_protect` | `addr: usize`, `length: usize`, `prot: usize` | 0 or negated errno | Change protection flags on a mapped region. Returns `EINVAL` if `addr` is not page-aligned, `ENOMEM` if any page is not mapped |

---

## Group: event (0x50–0x5F)

Waiting, timing, and synchronization.

| Number | Name | Parameters | Returns | Description |
|--------|------|-----------|---------|-------------|
| `0x50` | `event_create` | — | fd or negated errno | Create an event object [reserved, Phase 2] |
| `0x51` | `event_signal` | `handle: usize` | 0 or negated errno | Signal an event [reserved, Phase 2] |
| `0x52` | `event_wait` | `handle: usize` | 0 or negated errno | Wait for an event [reserved, Phase 2] |
| `0x53` | `event_wait_many` | `fds_ptr: usize`, `nfds: usize`, `timeout_ms: usize` | ready count or negated errno | Poll multiple fds. `fds_ptr` is an array of `PollFd`. `timeout_ms=0`: non-blocking; `usize::MAX`: infinite |
| `0x54` | `clock_gettime` | `clock_id: usize`, `tp: usize` | 0 or negated errno | Read the current time into a `Timespec` at `tp`. `clock_id`: `CLOCK_MONOTONIC` (0), `CLOCK_REALTIME` (1) |
| `0x55` | `clock_nanosleep` | `clock_id: usize`, `flags: usize`, `req_ptr: usize`, `rem_ptr: usize` | 0 or negated errno | Sleep for the duration in `*req_ptr`. Remaining time written to `*rem_ptr` if interrupted |
| `0x56` | `futex` | `addr: usize`, `op: usize`, `val: usize`, `timeout_ms: usize` | 0 or wakers count or negated errno | Fast userspace mutex. `op`: `FUTEX_WAIT` (sleep if `*addr==val`), `FUTEX_WAKE` (wake up to `val` waiters) |

### PollFd Structure

```c
struct PollFd {
    uint32_t fd;       // file descriptor to monitor
    uint16_t events;   // requested events: POLLIN, POLLOUT
    uint16_t revents;  // returned events: POLLIN, POLLOUT, POLLERR, POLLHUP, POLLNVAL
};
```

---

## Group: net (0x60–0x6F)

AF_UNIX socket operations.

| Number | Name | Parameters | Returns | Description |
|--------|------|-----------|---------|-------------|
| `0x60` | `socket` | `domain: usize`, `type_: usize`, `protocol: usize` | fd or negated errno | Create a socket. `domain`: `AF_UNIX` (1). `type_`: `SOCK_STREAM` (1). `protocol`: must be 0 |
| `0x61` | `bind` | `fd: usize`, `addr_ptr: usize`, `addr_len: usize` | 0 or negated errno | Bind a socket to a filesystem path. `addr_ptr` points to a `sockaddr_un` |
| `0x62` | `listen` | `fd: usize`, `backlog: usize` | 0 or negated errno | Mark a socket as listening |
| `0x63` | `accept` | `fd: usize`, `addr_ptr: usize`, `addr_len_ptr: usize` | new fd or negated errno | Accept a connection. Blocks until a peer connects. `addr_ptr` and `addr_len_ptr` may be null |
| `0x64` | `connect` | `fd: usize`, `addr_ptr: usize`, `addr_len: usize` | 0 or negated errno | Connect to a listening peer |
| `0x65` | `sendmsg` | `fd: usize`, `msg_ptr: usize`, `flags: usize` | bytes sent or negated errno | Send a message. `msg_ptr` is a POSIX `struct msghdr`. Supports `SCM_RIGHTS` for fd passing |
| `0x66` | `recvmsg` | `fd: usize`, `msg_ptr: usize`, `flags: usize` | bytes received or negated errno | Receive a message. Blocks if no data is available |
| `0x67` | `shutdown` | `fd: usize`, `how: usize` | 0 or negated errno | Shut down a connection. `how`: `SHUT_RD` (0), `SHUT_WR` (1), `SHUT_RDWR` (2) |

---

## Group: system (0xF0–0xFF)

System information and debugging.

| Number | Name | Parameters | Returns | Description |
|--------|------|-----------|---------|-------------|
| `0xF0` | `query` | `topic: usize`, `sub_id: usize`, `out_buf: usize`, `out_len: usize` | 0 or negated errno | Query system information. `topic` selects the information category; `out_buf` receives a typed response struct |
| `0xF1` | `debug_log` | `buf: usize`, `len: usize` | 0 or negated errno | Write a message to the kernel serial console. Available in all builds for debugging |

### Query Topics

| Topic constant | Value | Response struct | Description |
|---------------|-------|-----------------|-------------|
| `QUERY_MEMORY` | 0 | `MemoryInfo` | Total, free, and used physical memory in bytes |
| `QUERY_UPTIME` | 1 | `UptimeInfo` | Nanoseconds since boot |
| `QUERY_KERNEL_VERSION` | 2 | `KernelVersionInfo` | Kernel major/minor/patch and name string |
| `QUERY_PROCESSES` | 3 | `ProcessInfo` | Number of active processes |
| `QUERY_VMAPS` | 4 | `VmapEntry[]` | Virtual memory map of the calling process |
| `QUERY_CPU_INFO` | 5 | `CpuInfo` | Core count, feature flags, CPU model string |

---

## Common Structures

### Timespec

```c
struct Timespec {
    uint64_t tv_sec;   // seconds since boot (monotonic) or Unix epoch (real-time)
    uint64_t tv_nsec;  // nanoseconds within the current second (0–999,999,999)
};
```

### DirEntryInfo

```c
struct DirEntryInfo {
    uint8_t inode_type;   // 0=file, 1=dir, 2=chardev
    uint8_t name_len;     // length of the name in bytes
    uint8_t _pad[2];
    uint8_t name[60];     // UTF-8 name bytes (not NUL-terminated)
};
```

---

## Notes on Future Groups

The following syscall groups are planned for Phase 4 and Phase 5 but are not yet implemented. Their number assignments are reserved:

| Group | Range | Phase | Description |
|-------|-------|-------|-------------|
| `iommu` | 0x70–0x7F | 4 | IOMMU object management: `iommu_create_bti`, `bti_pin_memory`, `pmt_unpin` |
| `interrupt` | 0x80–0x8F | 4 | Hardware interrupt delivery: `interrupt_bind`, `interrupt_wait`, `interrupt_ack` |
| `resource` | 0x90–0x9F | 4 | MMIO/IO port resources: `resource_create_mmio`, `resource_map` |
| `pager` | 0xA0–0xAF | 5 | Demand paging protocol: `pager_create`, `pager_supply_pages`, `pager_op_range` |

The exact signatures for these groups will be documented when the corresponding phase begins implementation.
