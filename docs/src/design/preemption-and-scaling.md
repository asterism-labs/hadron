# Preemption & Scaling Architecture

This document describes the architectural decisions for preemption and executor scaling in Hadron. It establishes the design contracts that future phases must follow.

## Preemption Model

Hadron uses two layers of preemption with fundamentally different mechanisms.

### Userspace Preemption (Timer-Driven, Phase 9)

When a user process runs in ring 3, it can be preempted by the timer interrupt:

1. Timer interrupt fires while process runs in ring 3.
2. CPU traps to kernel, saves user registers on the interrupt frame.
3. Timer handler sets `PerCpu::preempt_current` flag.
4. Interrupt return path checks the flag; if set, returns to the executor instead of userspace.
5. `process_task` sees `UserspaceReturn::Preempted`, calls `yield_now().await`.
6. Task re-queued at back of Normal priority; another task gets to run.
7. When re-polled, `enter_userspace` restores saved user state via `iretq`.

```rust
enum UserspaceReturn {
    Syscall(SyscallArgs),
    Preempted,
    Fault(FaultInfo),
}

async fn process_task(process: Arc<Process>) {
    loop {
        match enter_userspace(&process) {
            UserspaceReturn::Syscall(args) => {
                match handle_syscall(&process, args).await {
                    SyscallResult::Continue(ret) => set_return_value(ret),
                    SyscallResult::Exit(status) => {
                        process.exit_notify.wake_all();
                        return;
                    }
                }
            }
            UserspaceReturn::Preempted => {
                // User state already saved. Yield to let other tasks run.
                yield_now().await;
            }
            UserspaceReturn::Fault(info) => {
                handle_fault(&process, info).await;
            }
        }
    }
}
```

The key insight: no Future is "interrupted". The async state machine never knew userspace was running. `enter_userspace()` is a regular function call that returns one of three variants. The Future's state machine sits on the heap, untouched during the entire time userspace executes.

### Kernel-Side Cooperative Preemption (Existing)

Kernel code cannot be preempted mid-function. Rust's ownership model (local variables on the stack, state machine mid-transition, possibly holding spinlocks) makes it unsafe to abandon a function mid-execution.

Instead, the kernel uses cooperative preemption:

- Futures yield at `.await` points.
- The `preempt_pending` flag is checked between task polls in the executor loop.
- Long kernel operations must insert `yield_now().await` every ~4 KiB of work.
- The lock-during-poll bug fix (see [Known Issues Fixed](#known-issues-fixed)) ensures timer interrupts actually fire during polls, making the budget check effective.

## Executor Scaling Path

### Current State (BSP-Only)

Single `LazyLock<Executor>` global with `BTreeMap` task storage. Sufficient for single-CPU development through Phase 11.

### Phase 12: Per-CPU Executors

Each CPU gets its own executor instance with local task storage:

```
Per-CPU Executor:
  +-------------------+
  | TaskSlab (local)  | <- O(1) insert/remove, no cross-CPU lock
  | ReadyQueues       | <- per-CPU, drained locally
  | WakeQueue (MPSC)  | <- lock-free push from remote CPUs/IRQs
  +-------------------+

Cross-CPU wakeup:
  Waker encodes target CPU-id -> push to target's WakeQueue -> send IPI
```

### Slab Task Storage (Replaces BTreeMap in Phase 12)

- Dense array indexed by slab key; O(1) insert/remove.
- Generational `TaskId` (upper bits = generation, lower bits = slab index) prevents ABA problems.
- Free list for O(1) allocation.
- Per-CPU slab eliminates cross-CPU lock contention during poll.

### Waker Encoding (Forward-Compatible from Phase 6)

The waker data pointer packs priority, CPU ID, and task ID into 64 bits:

```
Bit 63    62    61          56    55                             0
+----+----+----+----+----+----+----+----+-- ... --+----+----+----+
| P1 | P0 | C5 | C4 | C3 | C2 | C1 | C0 |        TaskId        |
+----+----+----+----+----+----+----+----+-- ... --+----+----+----+
  Priority      CPU ID (6 bits)          TaskId (56 bits)
```

- **Bits 63-62**: Priority (2 bits, 3 levels used)
- **Bits 61-56**: CPU ID (6 bits, supports up to 64 CPUs; hardcoded to 0 until Phase 12)
- **Bits 55-0**: TaskId (56 bits, practically unlimited)

The CPU ID field is reserved now to avoid a breaking encoding change when SMP arrives. When a waker fires on a remote CPU, it reads the target CPU ID from the waker data and pushes the task to that CPU's wake queue, then sends an IPI.

## WaitQueue Architecture

Two tiers of wait queues serve different parts of the kernel:

| Tier | Type | Capacity | Use Case |
|------|------|----------|----------|
| Frame | `WaitQueue` (ArrayVec) | 32 wakers | IRQ bottom-halves, low-level sync |
| Service | `HeapWaitQueue` (VecDeque) | Unbounded | Pipes, futexes, sockets, VFS |

### Condvar Pattern for Service-Layer Waiters (Phase 8+)

To prevent thundering-herd problems, service-layer waiters should use the condvar pattern:

```rust
// Instead of simple "wake = ready", check a condition:
loop {
    if condition() { return; }
    wait_queue.register_waker(cx.waker());
    if condition() { return; }  // re-check after registration
    return Poll::Pending;
}
```

`wake_all` wakes everyone, but only the task whose condition is satisfied proceeds; others re-register. The double-check (before and after registration) prevents the race where the condition becomes true between the check and the registration.

## Signal Delivery and Preemption Interaction

Signals are checked at two points:

1. **Syscall return** (existing plan): after `handle_syscall`, check `process.signals.dequeue()`.
2. **Preemption return** (new): when `UserspaceReturn::Preempted`, also check pending signals before re-entering userspace.

This means `SIGKILL` is delivered within one timer tick (~1ms) even to a tight userspace loop that makes no syscalls.

### Minimal Signal Set

| Signal | Number | Default Action |
|--------|--------|----------------|
| SIGINT | 2 | Terminate (Ctrl+C) |
| SIGKILL | 9 | Terminate (cannot be caught) |
| SIGSEGV | 11 | Terminate (invalid memory) |
| SIGPIPE | 13 | Terminate (broken pipe) |
| SIGTERM | 15 | Terminate |
| SIGCHLD | 17 | Ignore |
| SIGCONT | 18 | Continue (resume stopped process) |
| SIGSTOP | 19 | Stop (cannot be caught) |

`SIGINT`, `SIGSTOP`, and `SIGCONT` are essential for any interactive use and job control.

## Performance Contracts

These contracts define the cooperative expectations for kernel code:

| Contract | Value | Rationale |
|----------|-------|-----------|
| Maximum poll duration | ~1ms (one timer tick) | Between yield points; prevents starving other tasks |
| Yield point guideline | Every ~4 KiB of data | Insert `yield_now().await` in loops processing bulk data |
| Lock hold duration | <10 us | IrqSpinLock: no allocations, no I/O under lock |
| Future size budget | Warn if >4 KiB | Use `Box::pin()` at VFS/driver boundaries for large futures |

These are guidelines, not enforced limits. The executor's `preempt_pending` check provides a safety net: if a task polls for longer than one timer tick, the executor yields to the main loop after that poll completes.

## Known Issues Fixed

### Lock-During-Poll Bug

**Problem:** In the original executor, the `tasks` IrqSpinLock was held during the entire `future.poll()` call. Since IrqSpinLock disables interrupts (`cli`), this meant:

- Timer interrupts never fired during a poll, so `preempt_pending` was never set mid-poll.
- The budget check after poll was dead code (could only trigger between polls, not during).
- On SMP, other CPUs would spin waiting for the lock during every poll.
- A task polling for 10ms would block ALL interrupts for 10ms.

**Fix:** Restructured `poll_ready_tasks()` to remove the task from storage (brief lock), drop the lock, poll with interrupts enabled, then re-insert (brief lock). The `BTreeMap::remove` returning `None` protects against double-queue races (a waker fires while we're polling the same task).

### HeapWaitQueue O(n) wake_one

**Problem:** `HeapWaitQueue` used `Vec<Waker>` with `swap_remove(0)`, which is O(n) for the first element.

**Fix:** Replaced with `VecDeque<Waker>` using `pop_front()` for O(1) FIFO wake-one semantics.
