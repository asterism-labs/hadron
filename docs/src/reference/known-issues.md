# Known Issues

Tracked bugs and limitations discovered during development that have not yet
been addressed. Each entry includes the affected subsystem, a description,
and pointers to the relevant code.

## Process Management

### `waitpid(0)` always reaps the first child

`handle_wait` in `kernel/hadron-kernel/src/proc/mod.rs` with `pid = 0` ("wait
for any child") always picks `children_of(parent_pid)[0]` — the first child in
the list — rather than checking for any child that has already exited.

**Impact:** Init's wait loop only ever reaps the first-spawned shell. If a
later-spawned shell exits first, init will not notice until the first child
also exits.

**Fix:** Iterate `children_of(parent_pid)` and return the first zombie, or
subscribe a waker that fires when *any* child exits.

## Syscall / VFS

### `trap_io` leaks `Arc<dyn Inode>` references

In `kernel/hadron-kernel/src/syscall/vfs.rs`, `sys_vnode_read` and
`sys_vnode_write` resolve the inode into a local `Arc<dyn Inode>`. When the
fast path (`try_poll_immediate`) returns `Pending`, `trap_io()` is called,
which is `-> !` (longjmp via `restore_kernel_context`). The local `Arc` is
never dropped, so its reference count is never decremented.

**Impact:** Each blocking read/write syscall leaks one `Arc` strong count.
Over many syscalls the inode will never be freed even after all file
descriptors are closed. In practice the leak is slow (one per blocking I/O)
and the inodes are long-lived devfs entries, so it is not immediately
critical.

**Fix:** Either:
- Stash the `Arc` in a per-CPU slot that `process_task`'s TRAP_IO handler
  drops after the `.await` completes, or
- Restructure the syscall path so the `Arc` is moved into the future rather
  than held as a local across the longjmp boundary.

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
