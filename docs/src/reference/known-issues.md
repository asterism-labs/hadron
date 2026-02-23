# Known Issues

Tracked bugs and limitations discovered during development that have not yet
been addressed. Each entry includes the affected subsystem, a description,
and pointers to the relevant code.

## Process Management

### ~~`waitpid(0)` always reaps the first child~~ — **Fixed**

`handle_wait` with `pid = 0` now scans all children for the first zombie
using a `poll_fn` with double-check wakeup registration on every child's
`exit_notify`. Init correctly reaps whichever shell exits first.

## Syscall / VFS

### ~~`trap_io` leaks `Arc<dyn Inode>` references~~ — **Fixed**

`sys_vnode_read` and `sys_vnode_write` now explicitly `drop(inode)` before
calling `trap_io()`. The TRAP_IO handler in `process_task` re-fetches the
inode from the fd table, so it does not depend on the caller's Arc.

## Shell / Job Control

### No `WNOHANG` for `waitpid`

Background job reaping in the shell requires non-blocking `waitpid`, which
is not yet implemented. The shell can track background jobs but cannot poll
for their completion without blocking. A `WNOHANG` flag for `task_wait`
would allow the shell to reap finished background jobs at prompt time.

## Locking Discipline

### Lock ordering reference

The following lock ordering is enforced by convention (descending level
order — acquire higher levels first):

| Level | Lock                  | Type         | Location                     |
|------:|-----------------------|--------------|------------------------------|
|    14 | `Executor.tasks`      | IrqSpinLock  | `kernel/sched/src/executor.rs` |
|    13 | `Executor.ready_queues` | IrqSpinLock | `crates/core/hadron-core/src/sched.rs` |
|    10 | `TTY_LDISC`           | IrqSpinLock  | `kernel/hadron-kernel/src/tty/mod.rs` |
|    10 | `SCANCODE_BUF`        | IrqSpinLock  | `kernel/hadron-kernel/src/tty/mod.rs` |
|     4 | `PROCESS_TABLE`       | SpinLock     | `kernel/hadron-kernel/src/proc/mod.rs` |
|     4 | `fd_table`            | SpinLock     | `kernel/hadron-kernel/src/proc/mod.rs` |
|     1 | `HEAP`                | SpinLock     | `kernel/mm/src/heap.rs`      |
|     0 | `LOGGER`              | SpinLock     | `kernel/hadron-kernel/src/log.rs` |
|     0 | `TTY_WAKER`           | IrqSpinLock  | `kernel/hadron-kernel/src/tty/mod.rs` |
|     0 | `TTY_FBCON`           | SpinLock     | `kernel/hadron-kernel/src/tty/mod.rs` |
|     0 | `FBCON`               | SpinLock     | `kernel/hadron-kernel/src/drivers/fbcon/mod.rs` |

**Key rules:**
- Never call `waker.wake()` while holding a lock — it acquires
  `ready_queues` (level 13). Take the waker out first, release the lock,
  then invoke it. See `HeapWaitQueue::wake_all` for the correct pattern.
- `SpinLock::lock()` panics (with `hadron_lock_debug`) if called while any
  `IrqSpinLock` is held (`irq_lock_depth != 0`). The heap allocator uses
  `lock_unchecked()` to bypass this, since allocations may occur inside
  IrqSpinLock critical sections.
- `LOGGER` and `FBCON` are level 0 (no ordering check) because they sit at
  the bottom of the call chain — only ever acquired *inside* the logger's
  write path.
