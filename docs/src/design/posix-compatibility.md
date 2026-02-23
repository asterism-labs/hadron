# POSIX Compatibility

Hadron uses a **native handle-based syscall interface** with a planned
**userspace POSIX shim library** (hadron-libc) to translate standard POSIX
calls to native Hadron syscalls. The kernel implements only what cannot be
reasonably shimmed in userspace.

```
┌──────────────────────────────────┐
│        POSIX Application         │  ← calls fork(), open(), read(), etc.
├──────────────────────────────────┤
│    hadron-libc (POSIX shim)      │  ← translates to native Hadron syscalls
├──────────────────────────────────┤
│    Native Hadron Syscall ABI     │  ← SYSCALL instruction, handle-based
├──────────────────────────────────┤
│        Hadron Kernel             │
└──────────────────────────────────┘
```

## Current Status

The kernel now has **46 implemented syscalls** covering process management,
file I/O, memory mapping, signals, IPC, terminals, and threading. The
following table shows what has been implemented, grouped by priority tier.

### Implemented (P0 — Core)

| Kernel Syscall | POSIX Equivalent | Notes |
|----------------|-----------------|-------|
| `vnode_open` | `open()` | Supports O_APPEND, O_CLOEXEC, O_NONBLOCK, O_DIRECTORY, O_EXCL |
| `vnode_read` / `vnode_write` | `read()` / `write()` | Async via trap mechanism |
| `vnode_seek` | `lseek()` | SEEK_SET, SEEK_CUR, SEEK_END |
| `vnode_stat` | `fstat()` | Returns InodeStat |
| `vnode_fstatat` | `fstatat()` | AT_SYMLINK_NOFOLLOW support |
| `vnode_mkdir` | `mkdir()` | — |
| `vnode_unlink` | `unlink()` | — |
| `vnode_readdir` | `getdents()` | — |
| `handle_close` | `close()` | — |
| `handle_dup` | `dup2()` | — |
| `handle_dup_lowest` | `dup()` | Allocate lowest free fd |
| `task_exit` | `_exit()` | — |
| `task_spawn` | `posix_spawn()` | With fd_map, cwd, flags |
| `task_wait` | `waitpid()` | WNOHANG, WUNTRACED |
| `task_info` | `getpid()` | — |
| `task_getppid` | `getppid()` | — |
| `task_getcwd` / `task_chdir` | `getcwd()` / `chdir()` | Per-process CWD |
| `mem_map` / `mem_unmap` | `mmap()` / `munmap()` | Anonymous + device-backed |
| `mem_brk` | `brk()` | Program break management |
| `clock_gettime` | `clock_gettime()` | CLOCK_MONOTONIC |
| `clock_nanosleep` | `clock_nanosleep()` | Via trap mechanism |

### Implemented (P1 — Shell & Signals)

| Kernel Syscall | POSIX Equivalent | Notes |
|----------------|-----------------|-------|
| `task_execve` | `execve()` | In-place replacement via trap |
| `task_kill` | `kill()` | — |
| `task_sigaction` | `sigaction()` | SA_RESTART, SA_RESETHAND |
| `task_sigreturn` | `sigreturn()` | — |
| `task_sigprocmask` | `sigprocmask()` | SIG_BLOCK, SIG_UNBLOCK, SIG_SETMASK |
| `task_setpgid` / `task_getpgid` | `setpgid()` / `getpgid()` | — |
| `task_setsid` | `setsid()` | — |
| `handle_pipe` / `handle_pipe2` | `pipe()` / `pipe2()` | O_CLOEXEC support |
| `handle_fcntl` | `fcntl()` | F_DUPFD, F_GETFD/SETFD, F_GETFL/SETFL |
| `handle_ioctl` | `ioctl()` | TCGETS, TCSETS, TIOCGWINSZ, TIOCGPGRP |
| `handle_tcsetpgrp` / `handle_tcgetpgrp` | `tcsetpgrp()` / `tcgetpgrp()` | — |
| `vnode_rename` | `rename()` | — |
| `vnode_symlink` / `vnode_link` / `vnode_readlink` | `symlink()` / `link()` / `readlink()` | — |
| `vnode_truncate` | `ftruncate()` | — |

### Implemented (P2 — I/O Multiplexing & Terminals)

| Kernel Syscall | POSIX Equivalent | Notes |
|----------------|-----------------|-------|
| `event_wait_many` | `poll()` | Non-blocking poll; blocking deferred |
| termios ioctls | `tcgetattr()` / `tcsetattr()` | TCGETS/TCSETS/TCSETSW/TCSETSF |
| winsize ioctls | `TIOCGWINSZ` / `TIOCSWINSZ` | — |

### Implemented (P3 — Threads, Futex, PTY)

| Kernel Syscall | POSIX Equivalent | Notes |
|----------------|-----------------|-------|
| `task_clone` | `clone()` | CLONE_VM, CLONE_FILES, CLONE_SETTLS |
| `futex` | `futex()` | FUTEX_WAIT (async), FUTEX_WAKE |
| `/dev/ptmx` + `/dev/pts/N` | Pseudoterminals | Bidirectional buffers, termios |
| `Inode::on_open()` | — | Open-time inode substitution for ptmx |

## Future Work

The following features are needed for full POSIX application support but are
not yet implemented.

### High Priority — Needed for dash/busybox

| Feature | Description | Effort |
|---------|-------------|--------|
| `fork()` shim | Emulate fork+exec via `task_spawn` with fd_map in hadron-libc | Medium |
| `CLOCK_REALTIME` | RTC driver for wall-clock time; needed by `date`, `ls -l` | Medium |
| Blocking `event_wait_many` | Trap-based blocking poll with timeout | Medium |
| `SA_SIGINFO` / `siginfo_t` | Extended signal information | Medium |
| `wait4` with `WUNTRACED` | Stopped-child reporting for job control | Easy |
| `O_NOFOLLOW` for symlinks | Don't follow symlinks in open | Easy |
| `isatty()` support | Inode type check for terminal detection | Easy |
| `access()` | File permission check (shimmed via stat) | Easy |
| hadron-libc | POSIX shim library: errno, signal(), getuid/gid stubs, etc. | Large |

### Medium Priority — Needed for interactive programs

| Feature | Description | Effort |
|---------|-------------|--------|
| TTY raw mode | Line discipline honors `~ICANON` for byte-at-a-time input | Medium |
| `timer_create` / `setitimer` | Periodic timers (can be shimmed with nanosleep) | Medium |
| `mprotect` | Change page permissions on existing mappings | Medium |
| File locking (`F_SETLK`) | Advisory locking via fcntl | Medium |
| `CLOCK_REALTIME` in nanosleep | Absolute-time sleep for condition variable timeouts | Easy |

### Low Priority — Deferred

| Feature | Description | Effort |
|---------|-------------|--------|
| Socket subsystem | `socket`/`bind`/`listen`/`accept`/`connect`/`send`/`recv` | Very Large |
| `select()` / `epoll` | Can be shimmed on top of `event_wait_many` | Medium |
| Shared memory (`shmget`) | SysV shared memory segments | Medium |
| Real-time signals | Queued signals with `SA_SIGINFO` | Medium |
| `mremap` | Resize existing mappings | Medium |
| Multi-user permissions | Real uid/gid/mode enforcement | Large |

## Design Decisions

### No `fork()` in the kernel

Hadron uses `task_spawn` (like Fuchsia's `zx_process_create`) instead of
`fork()`. Fork requires CoW page table cloning — a major implementation
burden for a feature that modern programs avoid. The userspace shim can
emulate `fork()+exec()` patterns via `task_spawn` with fd remapping, but
programs that `fork()` without `exec()` (parallel computation in a copy
of the address space) are not supported.

### Thread model via `task_clone`

Threads are created with `task_clone(CLONE_VM | CLONE_FILES | ...)`, which
shares the parent's address space and file descriptor table via `Arc`.
The `Process` struct wraps shareable fields in `Arc<SpinLock<T>>` so that
threads and standalone processes use the same type. `execve` in a
multithreaded process is currently undefined — the address space is
replaced without killing sibling threads.

### Native poll, not POSIX poll

The native I/O multiplexing primitive is `event_wait_many`, which operates
on Hadron's fd/handle model. POSIX `poll()`, `select()`, and `epoll` will
be shimmed in hadron-libc by translating to `event_wait_many`.

## Verification Milestones

| Milestone | Test | Status |
|-----------|------|--------|
| M1: Static hello world | `write()` + `exit()` | Done |
| M2: File operations | open/read/write/seek/stat | Done |
| M3: Process management | Spawn child with piped stdin/stdout | Done |
| M4: dash shell boots | Builtins: cd, pwd, echo | Blocked on hadron-libc |
| M5: Shell pipelines | `ls \| grep foo \| wc -l` | Blocked on hadron-libc |
| M6: busybox coreutils | ls, cat, mkdir, rm, cp, mv, grep, find | Blocked on hadron-libc |
| M7: Interactive editing | vi/nano in terminal | Needs raw mode |
| M8: Multithreaded programs | pthreads, mutex, condvar | Needs hadron-libc pthreads |
| M9: Network utilities | curl, wget, DNS | Needs socket subsystem |
