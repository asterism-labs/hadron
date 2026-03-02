# Known Issues

Tracked bugs and limitations discovered during development that have not yet
been addressed. Each entry includes the affected subsystem, a description,
and pointers to the relevant code.

## POSIX Compatibility Limitations

### No `fork()` — spawn-only process model

Hadron does not implement `fork()`. The kernel uses `task_spawn` with an
explicit fd_map for process creation, similar to `posix_spawn`. Programs
that call `fork()` without immediately calling `exec()` (e.g., Apache
prefork, some daemons) cannot be supported. The hadron-libc shim will
translate `fork()+exec()` sequences into `task_spawn`, but fork-without-exec
is a fundamental limitation.

### `execve` in multithreaded processes is undefined

`task_execve` replaces the calling thread's address space but does **not**
kill sibling threads sharing the same address space via `task_clone`. This
violates POSIX semantics (where `execve` kills all threads in the thread
group). Programs must not call `execve` from a multithreaded process.

### `event_wait_many` is non-blocking only

The `event_wait_many` syscall (native `poll()`) currently only supports
`timeout_ms == 0` (non-blocking). Blocking poll with a timeout requires a
trap-based implementation similar to `futex_wait`, which is not yet done.
Programs needing blocking poll must spin with `nanosleep` in between.

### Futex uses virtual addresses, not physical

The futex implementation keys wait queues on user virtual addresses, not
physical addresses. This means futexes in shared memory mappings between
different processes (different page tables) will not work correctly — two
processes mapping the same physical page at different virtual addresses
will use different futex queues. This is fine for threads (which share
page tables via `CLONE_VM`) but limits cross-process futex use.

### PTY line discipline does not honor `~ICANON`

The pseudoterminal and TTY line discipline always operate in cooked mode.
Setting `~ICANON` in termios (raw mode) is accepted but has no effect on
the line discipline's buffering behavior. Interactive programs like `vi`,
`nano`, and `less` that require character-at-a-time input will not work
correctly until raw mode is implemented in the line discipline.

### Signal delivery lacks `SA_SIGINFO` / `siginfo_t`

Signal handlers receive only the signal number. The extended `siginfo_t`
structure (providing fault address, sending PID, etc.) is not supported.
Programs that install handlers with `SA_SIGINFO` will get `EINVAL`.

### No network socket subsystem

`AF_UNIX` stream sockets are implemented (Wayland compositor transport).
Network sockets (`AF_INET`/`AF_INET6`) are not. All TCP/UDP syscalls
(`socket(AF_INET, ...)`, `getaddrinfo`, etc.) return `ENOSYS`. This blocks
`curl`, `wget`, DNS resolution, and any network-aware program.

**Planned**: Network sockets remain deferred pending a full networking stack.

### No `mprotect`

Page permissions cannot be changed after mapping. `mprotect()` returns `ENOSYS`.
This blocks Mesa's shader JIT (which mmaps RW pages, writes shader code, then
mprotects to RX) and other JIT-based runtimes.

**Planned**: `sys_mem_protect` syscall as part of the graphics stack prerequisites.
See [Mesa & Vulkan](../features/mesa-vulkan.md).

## Locking Discipline

### Lock ordering reference

The following lock ordering is enforced by convention (descending level
order — acquire higher levels first):

| Level | Lock                  | Type         | Location                     |
|------:|-----------------------|--------------|------------------------------|
|    14 | `Executor.tasks`      | IrqSpinLock  | `kernel/sched/src/executor.rs` |
|    13 | `Executor.ready_queues` | IrqSpinLock | `kernel/core/src/sched.rs` |
|    10 | `TTY_LDISC`           | IrqSpinLock  | `kernel/kernel/src/tty/mod.rs` |
|    10 | `SCANCODE_BUF`        | IrqSpinLock  | `kernel/kernel/src/tty/mod.rs` |
|     4 | `PROCESS_TABLE`       | SpinLock     | `kernel/kernel/src/proc/mod.rs` |
|     4 | `fd_table`            | SpinLock (Arc) | `kernel/kernel/src/proc/mod.rs` |
|     4 | `address_space`       | SpinLock (Arc) | `kernel/kernel/src/proc/mod.rs` |
|     3 | `unix_socket`         | SpinLock     | `kernel/kernel/src/net/unix.rs` |
|     3 | `unix_registry`       | SpinLock     | `kernel/kernel/src/net/unix.rs` |
|     3 | `unix_a2b` / `unix_b2a` | SpinLock  | `kernel/kernel/src/net/unix.rs` |
|     2 | `FUTEX_TABLE`         | SpinLock     | `kernel/kernel/src/ipc/futex.rs` |
|     2 | `PTY_SLAVES`          | SpinLock     | `kernel/kernel/src/tty/pty.rs` |
|     1 | `HEAP`                | SpinLock     | `kernel/mm/src/heap.rs`      |
|     0 | `LOGGER`              | SpinLock     | `kernel/kernel/src/log.rs` |
|     0 | `TTY_WAKER`           | IrqSpinLock  | `kernel/kernel/src/tty/mod.rs` |
|     0 | `FBCON`               | SpinLock     | `kernel/kernel/src/drivers/fbcon/mod.rs` |

**Key rules:**
- Never call `waker.wake()` while holding a lock — it acquires
  `ready_queues` (level 13). Take the waker out first, release the lock,
  then invoke it. See `HeapWaitQueue::wake_all` for the correct pattern.
- When closing an fd that may be a unix socket, use `close_take()` and drop
  the result **after** releasing `fd_table`. `Drop for UnixSocket` acquires
  `unix_socket` (level 3), which is lower than `fd_table` (level 4), so
  dropping inside the lock violates ordering.
- `SpinLock::lock()` panics (with `hadron_lock_debug`) if called while any
  `IrqSpinLock` is held (`irq_lock_depth != 0`). The heap allocator uses
  `lock_unchecked()` to bypass this, since allocations may occur inside
  IrqSpinLock critical sections.
- `LOGGER` and `FBCON` are level 0 (no ordering check) because they sit at
  the bottom of the call chain — only ever acquired *inside* the logger's
  write path.
