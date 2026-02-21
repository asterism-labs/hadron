# Executor Architecture

This document describes the design of Hadron's kernel async executor, including its priority model, waker encoding, SMP migration path, and the syscall bridge trade-off analysis.

## Priority Executor Design

The executor organizes kernel tasks into three strict priority tiers, always drained highest-first:

| Tier | Use Case | Examples |
|------|----------|---------|
| `Critical` | Interrupt bottom-halves, hardware event completion | Serial RX processing, keyboard event dispatch |
| `Normal` | Kernel services, driver tasks | Serial echo, async I/O |
| `Background` | Housekeeping, statistics | Heartbeat, log flushing, memory compaction |

Within each tier, tasks are scheduled in FIFO order. The executor always drains Critical before Normal, Normal before Background.

### Background Starvation Prevention

To prevent Background tasks from being starved indefinitely by Normal tasks, the executor polls at least one Background task every 100 consecutive Normal pops (~100ms at typical task rates) even if Normal tasks are still pending.

### Why Not Multi-Level Feedback?

MLFQ detects CPU-bound vs I/O-bound tasks via time slice exhaustion. Hadron's executor is cooperative — tasks yield at `.await` points, so there is no time slice to exhaust. Strict priority with FIFO per-tier is simpler and matches kernel task semantics exactly.

## Waker Encoding

Priority is packed into the `RawWaker` data pointer alongside `TaskId`:

```
Bit 63    62    61                                            0
┌────┬────┬────────────────────────────────────────────────────┐
│ P1 │ P0 │                   TaskId (62 bits)                 │
└────┴────┴────────────────────────────────────────────────────┘
```

- **Bits 63-62**: Priority (2 bits → 4 levels, 3 used)
- **Bits 61-0**: TaskId (62 bits → practically unlimited)

When a waker fires, it extracts the priority from the data pointer and pushes the task into the correct priority queue without any lock-based metadata lookup.

## SMP Migration Path

### Current State

Single `LazyLock<Executor>` global — BSP only.

### SMP-Ready Abstraction

The `CpuLocal<T>` wrapper indexes a static array by CPU ID. For now, `MAX_CPUS = 1`. When SMP arrives (Phase 12):

1. Increase `MAX_CPUS`
2. Replace `LazyLock<Executor>` with `CpuLocal<Executor>`
3. Each AP boots into its own `executor().run()` loop
4. Wakers encode target CPU in upper bits for cross-CPU wake (+ IPI)

The `CpuLocal<T>` type is defined in `hadron_kernel::percpu` and is available today but not yet used for the executor.

### Waker Encoding Expansion

The waker data pointer has been expanded to reserve 6 bits (61-56) for CPU ID, reducing TaskId from 62 to 56 bits. This prevents a breaking encoding change when SMP arrives. See [Preemption & Scaling](preemption-and-scaling.md#waker-encoding-forward-compatible-from-phase-6) for the full encoding layout.

### Per-CPU Slab Storage

Phase 12 will replace the `BTreeMap` task storage with a per-CPU slab allocator for O(1) insert/remove and zero cross-CPU lock contention during polls. See [Preemption & Scaling](preemption-and-scaling.md#slab-task-storage-replaces-btreemap-in-phase-12) for design details.

## Syscall Bridge Analysis

Three models were analyzed for how userspace threads (POSIX threads) interact with the kernel async executor. Model C (Hybrid) is recommended and extended with a `UserspaceReturn::Preempted` variant for timer-driven preemption of userspace code — see [Preemption & Scaling](preemption-and-scaling.md#userspace-preemption-timer-driven-phase-9) for the full design.

### Model A — Async Bridge

Convert blocking syscalls into kernel futures. User thread suspends, executor polls the future, resumes thread on completion.

**Pros:**
- Memory efficient (no per-thread kernel stack)
- Leverages existing async infrastructure
- I/O multiplexing "for free"

**Cons:**
- Complex user/kernel state management
- No deep kernel recursion
- Harder to debug

### Model B — Blocking Kernel Threads

Each user thread has a dedicated kernel stack and blocks on WaitQueue.

**Pros:**
- Simple programming model, familiar (Linux-like)
- Debuggable kernel stack traces

**Cons:**
- 64 KiB kernel stack per thread (1000 threads = 64 MiB)
- Doesn't leverage async executor
- Two scheduling systems to maintain

### Model C — Hybrid (Recommended)

Fast syscalls (`getpid`, `clock_gettime`, `brk`) run synchronously and return immediately. I/O syscalls (`read`, `write`, `poll`) try non-blocking first, then suspend the user thread and create an async kernel future.

**Pros:**
- Best of both worlds
- Incremental: start fully sync in Phase 7, add async I/O later
- Leverages existing executor

**Cons:**
- Two code paths per syscall category

### Recommendation

**Model C** is recommended because:
- Aligns with framekernel's safe-services constraint (Rust `Future` is safe, manual thread blocking is not)
- Incremental: Phase 7 can be fully synchronous, async bridge added later
- Per-CPU executors naturally handle the per-CPU syscall conversion path

## Driver Integration

Drivers already use `IrqLine` + `WaitQueue` for async IRQ bridging. With the priority executor:

- **Driver interrupt handler tasks** spawn as `Priority::Critical`
- **Driver service tasks** (echo loops, protocol processing) spawn as `Priority::Normal`
- No changes to `IrqLine`, `WaitQueue`, or `KernelServices` needed — priority is set at spawn site

## Known Issues Fixed

### Lock-During-Poll Bug

The original `poll_ready_tasks()` held the `tasks` IrqSpinLock during the entire `future.poll()` call, disabling all interrupts for the duration of every poll. This made the `preempt_pending` budget check dead code and would cause severe contention on SMP. Fixed by restructuring the function to remove the task from storage (brief lock), drop the lock before polling, then re-insert after (brief lock). See [Preemption & Scaling](preemption-and-scaling.md#known-issues-fixed) for full details.
