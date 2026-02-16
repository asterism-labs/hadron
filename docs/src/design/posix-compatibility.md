# POSIX Compatibility

> **Note:** This document describes an earlier design direction. Hadron has since adopted a **task-centric, capability-based** architecture that does not target POSIX compatibility. See [Task-Centric OS Design](task-centric-design.md) for the current design goals. This document is retained for historical reference.

Hadron takes an **incremental approach** to POSIX compatibility — implementing the most critical interfaces first, then expanding based on what applications actually need.

## Strategy

Rather than attempting full POSIX compliance upfront (which would require hundreds of syscalls and extensive infrastructure), Hadron:

1. **Starts with core syscalls** (~50) that cover basic program execution
2. **Adds syscalls on demand** as new features are implemented
3. **Uses Linux syscall numbers** for easy testing with existing tools
4. **Provides a userspace compatibility library** (future) that can paper over gaps

## Priority Syscall List

### Tier 1: Minimum Viable (Phase 7-9)

These syscalls are needed for any userspace program to run:

| Syscall | Purpose | Why Essential |
|---------|---------|---------------|
| `read` | Read from fd | Basic I/O |
| `write` | Write to fd | Basic I/O, console output |
| `open` | Open file | File access |
| `close` | Close fd | Resource cleanup |
| `exit` | Terminate process | Process lifecycle |
| `brk` | Adjust heap | Memory allocation (malloc) |
| `mmap` | Map memory | Memory allocation, ELF loading |
| `munmap` | Unmap memory | Memory cleanup |
| `getpid` | Get PID | Process identity |

### Tier 2: Filesystem (Phase 8)

| Syscall | Purpose |
|---------|---------|
| `stat` / `fstat` | File metadata |
| `lseek` | Seek in file |
| `readdir` / `getdents` | List directory |
| `mkdir` | Create directory |
| `unlink` | Delete file |
| `rename` | Rename file |
| `dup` / `dup2` | Duplicate fd |
| `fcntl` | File control |
| `ioctl` | Device control |
| `access` | Check permissions |
| `chdir` / `getcwd` | Working directory |

### Tier 3: Process Management (Phase 11)

| Syscall | Purpose |
|---------|---------|
| `fork` | Create child process |
| `execve` | Replace process image |
| `waitpid` / `wait4` | Wait for child |
| `kill` | Send signal |
| `sigaction` | Set signal handler |
| `sigprocmask` | Block/unblock signals |
| `pipe` / `pipe2` | Create pipe |
| `getppid` | Get parent PID |
| `getuid` / `getgid` | Get user/group ID |
| `setuid` / `setgid` | Set user/group ID |

### Tier 4: Advanced (Phase 12-15)

| Syscall | Purpose | Phase |
|---------|---------|-------|
| `socket` / `bind` / `listen` / `accept` | Networking | 13 |
| `connect` / `send` / `recv` | Networking | 13 |
| `select` / `poll` / `epoll` | I/O multiplexing | 13 |
| `clone` | Thread creation | 14 |
| `futex` | Fast userspace mutex | 14 |
| `clock_gettime` (vDSO) | High-perf time | 15 |
| `mprotect` | Change page permissions | — |
| `mremap` | Resize mapping | — |
| `shmget` / `shmat` | Shared memory | — |

## What We Won't Implement (Initially)

Some POSIX features are deprioritized because they're rarely needed or have better alternatives:

| Feature | Status | Reason |
|---------|--------|--------|
| `System V IPC` (msgget, semget) | Deferred | Modern code uses futex, pipes, or sockets |
| `aio_*` (POSIX AIO) | Deferred | io_uring is the modern replacement |
| `pthread_*` (POSIX threads) | Userspace only | Implemented in libc using `clone`/`futex` |
| `terminal/pty` | Deferred | Complex; basic serial console suffices initially |
| `POSIX realtime signals` | Deferred | Regular signals cover most needs |
| `setitimer` / `timer_create` | Deferred | `clock_gettime` + `nanosleep` suffice |

## Compatibility Testing

The practical test for POSIX compatibility is: **can we run real programs?**

### Target Applications (Ordered by Complexity)

1. **Custom init** — minimal `write()` + `exit()` (Phase 9)
2. **busybox** — statically linked, ~100 common Unix utilities (Phase 11)
3. **dash** — lightweight POSIX shell (Phase 11)
4. **lua** — embeddable scripting language (Phase 11)
5. **gcc/tcc** — self-hosting compiler (long-term goal)

### Testing Approach

```bash
# Cross-compile a static binary for Hadron
# (Initially using x86_64-unknown-linux-musl until we have our own target)
musl-gcc -static -o init test.c

# Package into initramfs
echo init | cpio -o -H newc > initramfs.cpio

# Boot and test
cargo xtask run -- -initrd initramfs.cpio
```

## POSIX Deviations

Where POSIX doesn't serve us well, Hadron will deviate:

| Area | POSIX Way | Hadron Way | Rationale |
|------|-----------|------------|-----------|
| Error codes | `errno` global | Return value encoding | Thread-safe, no TLS needed initially |
| Signal handling | Complex semantics | Simplified subset | Full POSIX signals are a huge implementation burden |
| File permissions | uid/gid/mode | Same (initially) | Standard model works well enough |
| Process groups | Full job control | Basic support first | Shell job control is complex |

The userspace compatibility library will bridge these gaps where needed, translating Hadron's native interface to standard POSIX behavior.

## Growth Summary

| Milestone | ~Syscall Count | Applications Supported |
|-----------|---------------|----------------------|
| Phase 9 (first userspace) | ~15 | Custom test binaries |
| Phase 11 (fork/exec/pipe) | ~50 | Simple shell, busybox |
| Phase 13 (networking) | ~80 | Network utilities |
| Phase 15 (mature) | ~120 | Most command-line applications |
| Long-term | ~200 | General-purpose Unix environment |
