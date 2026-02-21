# Async Executor

Hadron uses a cooperative async executor as its core scheduling mechanism.
Instead of a traditional preemptive thread scheduler, all kernel work runs
as `Future<Output = ()> + Send + 'static` tasks that yield at `.await`
points. Each CPU runs its own executor instance, and tasks are organized
into three strict priority tiers.

The implementation lives under `kernel/hadron-kernel/src/sched/`, with
supporting types in `kernel/hadron-kernel/src/task.rs`.

## Key Types

| Type | Module | Purpose |
|------|--------|---------|
| `Executor` | `sched/executor.rs` | Per-CPU task executor; owns task storage and ready queues |
| `TaskId` | `task.rs` | Unique 64-bit task identifier (`TaskId(u64)`) |
| `Priority` | `task.rs` | Three-tier enum: `Critical`, `Normal`, `Background` |
| `TaskMeta` | `task.rs` | Task metadata: name, priority, CPU affinity |
| `ReadyQueues` | `sched/executor.rs` | Priority-aware FIFO queues with starvation prevention |
| `TaskEntry` | `sched/executor.rs` | Internal: pinned boxed future plus metadata |
| `CpuLocal<T>` | `percpu.rs` | Per-CPU storage wrapper indexed by CPU ID |

## Priority Tiers

The `Priority` enum (`task.rs`) defines three tiers, represented as `#[repr(u8)]`:

```rust
pub enum Priority {
    Critical   = 0,  // Interrupt bottom-halves, hardware event completion
    Normal     = 1,  // Kernel services and device drivers
    Background = 2,  // Housekeeping: memory compaction, log flushing, statistics
}
```

`Priority::COUNT` is `3`. The `from_u8` constructor maps unknown values to
`Normal`, making it safe for deserialization from packed waker data.

### Scheduling Policy

The `ReadyQueues` struct maintains one `VecDeque<TaskId>` per priority tier.
The `pop()` method enforces a strict ordering:

1. **Critical always first.** If the Critical queue is non-empty, it is
   drained before any other tier runs. Popping a Critical task resets the
   Normal streak counter.

2. **Normal next, with starvation prevention.** Normal tasks run in FIFO
   order. A `normal_streak` counter tracks how many consecutive Normal pops
   have occurred while Background tasks are waiting. Once the streak reaches
   `BACKGROUND_STARVATION_LIMIT` (100), one Background task is promoted
   before Normal resumes.

3. **Background last.** Background tasks only run when Critical and Normal
   queues are empty, or when the starvation limit forces one through.

This design ensures that latency-sensitive work (interrupt bottom-halves)
always runs immediately, normal kernel services get fair scheduling, and
background housekeeping makes progress without starving.

## Waker Encoding

Source: `sched/waker.rs`

Hadron uses a zero-allocation waker scheme. The `RawWaker` data pointer is
not a heap pointer but a packed 64-bit integer encoding three fields:

```
Bit layout (64-bit data pointer):
  Bits 63-62:  Priority   (2 bits, 3 levels used)
  Bits 61-56:  CPU ID     (6 bits, supports up to 64 CPUs)
  Bits 55-0:   TaskId     (56 bits)
```

The packing and unpacking functions:

```rust
fn pack(id: TaskId, priority: Priority) -> *const () {
    let cpu_id = crate::percpu::current_cpu().get_cpu_id() as u64;
    let packed = ((priority as u64) << 62) | (cpu_id << CPU_SHIFT) | (id.0 & ID_MASK);
    packed as *const ()
}

fn unpack(data: *const ()) -> (TaskId, Priority, u32) {
    let raw = data as u64;
    let priority = Priority::from_u8((raw >> 62) as u8);
    let cpu_id = ((raw >> CPU_SHIFT) & CPU_MASK) as u32;
    let id = TaskId(raw & ID_MASK);
    (id, priority, cpu_id)
}
```

The `RawWakerVTable` functions:

- **`clone`**: Returns a new `RawWaker` with the same data pointer (data is `Copy`).
- **`wake` / `wake_by_ref`**: Unpacks the data, pushes the `TaskId` onto the
  **originating CPU's** executor ready queue (not the current CPU's), and
  sends a wakeup IPI if the target is a different CPU.
- **`drop`**: No-op, since the packed data has no allocation to free.

The key insight is that wakers always target the CPU where the task was last
polled. When a task is woken from a different CPU (e.g., an interrupt
handler on CPU 1 completing I/O for a task on CPU 0), the waker pushes the
task ID into CPU 0's ready queue and sends an IPI to wake CPU 0 from HLT.

## Executor Main Loop

Source: `sched/executor.rs`

### Structure

Each CPU's `Executor` owns:

- **`tasks`**: An `IrqSpinLock<BTreeMap<TaskId, TaskEntry>>` mapping task IDs
  to their pinned futures and metadata.
- **`ready_queues`**: An `IrqSpinLock<ReadyQueues>` holding the priority-aware
  FIFO queues of task IDs that are ready to poll.
- **`next_id`**: An `AtomicU64` counter for generating unique task IDs.

Executors are stored in a `CpuLocal<LazyLock<Executor>>` static, initialized
on first access per CPU.

### The `run()` Loop

The `Executor::run()` method is called once per CPU and never returns:

```
loop {
    1. poll_ready_tasks()    -- drain ready queues, poll tasks
    2. try_steal()           -- attempt work stealing from other CPUs
    3. enable_and_hlt()      -- halt until next interrupt
}
```

**Step 1: `poll_ready_tasks()`** pops task IDs from the ready queues in
priority order. For each task:

1. Pop `(priority, id)` from the ready queues (lock acquired briefly).
2. Create a `Waker` via `task_waker(id, priority)` encoding the current CPU.
3. Remove the `TaskEntry` from the task map (brief lock, then released).
4. Poll the future with interrupts enabled -- this is critical because timer
   interrupts can fire during `future.poll()`, enabling budget-based
   preemption detection.
5. If `Poll::Ready`, the task is complete and not re-inserted.
6. If `Poll::Pending`, the entry is placed back into the task map.
7. After each poll, check `preempt_pending()`. If the timer interrupt set
   the flag during polling, clear it and break out of the loop to allow
   the executor to re-evaluate (work stealing, halt, etc.).

The temporary removal of the task entry from the `tasks` map during polling
is an intentional design choice. It ensures the `IrqSpinLock` is not held
during `future.poll()`, which can run for an arbitrary amount of time. If a
waker fires while the task is out for polling, the task ID lands in the
ready queue; the next iteration will find `tasks.remove(&id)` returns
`None` (the entry is still being polled), which is harmless -- the task
will be picked up on a subsequent pass after re-insertion.

**Step 2: Work stealing.** If no local tasks are ready, `smp::try_steal()`
is called before halting. See the [SMP section](#smp-and-work-stealing)
below.

**Step 3: Halt.** On x86\_64, the CPU executes `sti; hlt` atomically via
`enable_and_hlt()`, sleeping until the next interrupt (timer tick, device
IRQ, or wakeup IPI). After the interrupt fires, interrupts are disabled
again and the loop restarts.

### Budget-Based Preemption

Source: `sched/mod.rs`

A per-CPU `AtomicBool` flag, `PREEMPT_PENDING`, provides cooperative
preemption budgeting:

- **`set_preempt_pending()`** is called from the timer interrupt handler on
  each tick.
- **`preempt_pending()`** is checked after each task poll in
  `poll_ready_tasks()`.
- **`clear_preempt_pending()`** resets the flag when the executor breaks out
  of its polling loop.

This works because the task entry is removed from the task map before
polling, allowing interrupts to be enabled during `future.poll()`. The
timer interrupt fires, sets the preempt flag, and the next iteration of the
polling loop notices and yields to the main executor loop. The net effect is
that no single task can monopolize the CPU across a timer tick boundary,
even if it does substantial work between `.await` points.

## Timer Integration

Source: `sched/timer.rs`

The timer subsystem provides sleep waker registration using a min-heap
priority queue.

### Sleep Queue

A global `IrqSpinLock<BinaryHeap<Reverse<SleepEntry>>>` stores pending
sleep entries, each containing a deadline (in timer ticks) and a cloned
`Waker`. The `Reverse` wrapper gives min-heap behavior so the earliest
deadline is always at the top.

### Registration

When a task calls `sleep_ticks(n)` or `sleep_ms(ms)` (from
`sched/primitives.rs`), the `SleepFuture` computes a deadline from the
current tick count and, on each poll where the deadline has not passed,
calls `register_sleep_waker(deadline, waker)` to enqueue itself.

### Expiration

The timer interrupt handler calls `wake_expired(current_tick)` on every
tick. This function pops all entries from the heap whose deadline is at or
before the current tick, calling `waker.wake()` on each. Because the waker
encodes the target CPU, this correctly re-queues the task on the right
executor and sends an IPI if needed.

## SMP and Work Stealing

Source: `sched/smp.rs`

### IPI Wakeup

When a waker pushes a task to a remote CPU's ready queue, it also calls
`send_wake_ipi(target_cpu)`. This translates the logical CPU ID to a
physical APIC ID (via the `CPU_APIC_IDS` table populated during bootstrap)
and sends an IPI on the `IPI_WAKE_VECTOR` (vector 240). The handler is
intentionally empty -- the interrupt itself breaks the target CPU out of
`enable_and_hlt()`, causing it to re-enter the executor loop and discover
the newly enqueued task.

### Work Stealing

When a CPU's executor has no local ready tasks, it calls `try_steal()`
before halting. The stealing algorithm:

1. Skip if there is only one CPU online.
2. Pick a pseudo-random start offset (based on the current timer tick) to
   distribute stealing pressure across victims and avoid thundering herd.
3. Iterate over other CPUs. For each victim:
   - Call `victim_executor.steal_task()`, which uses `try_lock` on both the
     ready queues and the task map to avoid blocking the victim.
   - Steal from the **back** of the victim's ready queue (coldest task),
     preserving locality for the victim's hot (front) tasks.
   - Only Normal and Background tasks are stolen -- Critical tasks are
     never migrated.
   - If the task entry cannot be found (it is being polled or the lock is
     contended), the task ID is put back into the victim's queue.
4. If a task is stolen, the caller inserts it into their local task map and
   ready queue. On the next poll, the waker will encode the new CPU's ID,
   completing the migration transparently.

### Lock Ordering

The `steal_task()` method acquires locks in the order: ready queues first,
then task map. This is the **opposite** of `spawn_with_meta()` (task map
first, then ready queues). Both use `try_lock` in the steal path to avoid
deadlock.

## Spawning APIs and Task Lifecycle

Source: `sched/mod.rs`

The module provides convenience functions that delegate to the current CPU's
executor:

| Function | Priority | Description |
|----------|----------|-------------|
| `sched::spawn(future)` | Normal | Spawn with default metadata |
| `sched::spawn_with(future, meta)` | (from meta) | Spawn with explicit `TaskMeta` |
| `sched::spawn_critical(name, future)` | Critical | Interrupt bottom-halves, HW events |
| `sched::spawn_background(name, future)` | Background | Housekeeping, statistics |

### Task Lifecycle

1. **Spawn**: A future is boxed, pinned, and stored in the executor's task
   map with a new `TaskId`. The ID is also pushed onto the appropriate
   priority queue, making the task immediately ready.

2. **Poll**: The executor pops the task from the ready queue, creates a
   waker encoding the task's ID/priority/CPU, removes the entry from the
   task map, and calls `future.poll()`. If `Pending`, the entry is
   re-inserted into the task map (but not the ready queue -- the task will
   only be re-queued when its waker fires).

3. **Wake**: When an event completes (timer expiry, I/O completion, explicit
   `wake_by_ref`), the waker pushes the task ID onto the originating CPU's
   ready queue and sends an IPI if cross-CPU.

4. **Completion**: When `future.poll()` returns `Ready(())`, the task entry
   is simply not re-inserted. The `TaskEntry` (and its boxed future) is
   dropped, freeing memory.

5. **Migration**: If work stealing moves a task to another CPU, the entire
   `TaskEntry` is transferred. The new waker created on the next poll
   encodes the new CPU, so future wakeups target the correct executor.

### TaskMeta Builder

`TaskMeta` supports a const builder pattern:

```rust
let meta = TaskMeta::new("disk-irq-handler")
    .with_priority(Priority::Critical)
    .with_affinity(0);  // Pin to CPU 0
```

The `affinity` field (`Option<u32>`) is reserved for future use -- currently
tasks are not automatically pinned, but work stealing respects it once
implemented.

## Async Primitives

Source: `sched/primitives.rs`

### yield\_now

```rust
pub async fn yield_now() { ... }
```

Returns `Pending` once (calling `waker.wake_by_ref()` to re-queue
immediately), then `Ready` on the next poll. This is the primary
cooperative yield point for long-running kernel tasks.

### sleep\_ticks / sleep\_ms

```rust
pub async fn sleep_ticks(ticks: u64) { ... }
pub async fn sleep_ms(ms: u64) { ... }
```

Computes a deadline from the current tick count, then registers a waker
with the timer subsystem. The task is not re-queued until the deadline
expires and `wake_expired()` fires the waker. At 1 kHz timer frequency,
1 tick = 1 ms.

### join

```rust
pub fn join<A: Future, B: Future>(a: A, b: B) -> Join<A, B>
```

Polls two futures concurrently within a single task, returning
`(A::Output, B::Output)` when both complete. Uses structural pinning.

### select

```rust
pub fn select<A: Future, B: Future>(a: A, b: B) -> Select<A, B>
```

Polls two futures concurrently, returning `Either::Left(A::Output)` or
`Either::Right(B::Output)` for whichever completes first. The losing
future is dropped.

## block\_on: Sync-Async Bridge

Source: `sched/block_on.rs`

```rust
pub fn block_on<T>(future: impl Future<Output = T>) -> T
```

A blocking bridge for synchronous code that needs to call async operations.
Uses a no-op waker and busy-waits with `sti; hlt; cli` between polls,
yielding to interrupts (disk IRQs, timer) without pure spin-looping. This
is explicitly a temporary bridge for synchronous filesystem crates
(`hadris-*`) that do not yet support native async I/O.

## Architectural Diagram

```
                         Timer Interrupt
                              |
                              v
                    set_preempt_pending()
                              |
           +------------------+------------------+
           |             Per-CPU Executor         |
           |                                      |
           |  +-----------+  +-----------+        |
           |  | Critical  |  |   Task    |        |
           |  |   Queue   |  |   Map     |        |
           |  +-----------+  | (BTreeMap)|        |
           |  | Normal    |  +-----------+        |
           |  |   Queue   |       |               |
           |  +-----------+       | poll()        |
           |  | Background|       v               |
           |  |   Queue   |  [Future::poll]       |
           |  +-----------+       |               |
           |       ^              | Pending?      |
           |       |              v               |
           |       +--- waker.wake() ---+         |
           |                            |         |
           +------- work stealing ------+---------+
                         ^
                         |
                    Other CPUs
```
