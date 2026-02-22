# hadron-sched

Async cooperative task scheduler for the Hadron kernel. This crate provides per-CPU executors that poll kernel tasks as `Future<Output = ()>` instances. Tasks yield at `.await` points, and a per-CPU budget-based preemption flag (set by the timer interrupt) ensures fairness even when a task polls for an extended period between awaits. Architecture-specific behavior -- IPI wakeup, halt/WFI idle, and work-stealing routing -- is injected from the kernel glue layer via trait implementations and registered callbacks.

## Features

- **Per-CPU async executor** -- each CPU runs its own `Executor` instance; tasks are spawned on the current CPU and stay there unless migrated by work stealing; the executor's main loop polls ready tasks, attempts work stealing when idle, then halts until the next interrupt
- **Three-tier priority scheduling** -- tasks are organized into Critical (interrupt bottom-halves, hardware events), Normal (default), and Background (housekeeping, statistics) priorities; the executor always drains higher-priority tiers before lower ones
- **Waker-based ready queue** -- tasks are only polled when their waker has been invoked; the waker encodes the originating CPU ID so cross-CPU wakeups push the task back to its home executor via IPI
- **Work stealing** -- when a CPU's local queue is empty, it attempts to steal a task from another CPU's executor (back of queue to preserve locality); Critical tasks are never stolen
- **Timer-based sleep queue** -- sleeping tasks register a waker and deadline tick; the timer interrupt handler calls `wake_expired` each tick to wake tasks whose deadline has passed, with bounded batch draining to keep the ISR stack-allocated
- **Async scheduling primitives** -- `yield_now` for cooperative yielding, `sleep_ticks` and `sleep_ms` for timer-based delays, `join` for concurrent two-future completion, and `select` for racing two futures
- **Preemption flag** -- a per-CPU atomic flag set by the timer interrupt; the executor checks it between task polls and yields control to the main loop, allowing preempted ring-3 processes to be re-queued without starving other tasks
- **Convenience spawn functions** -- `spawn` (Normal priority), `spawn_critical` (named Critical task), and `spawn_background` (named Background task) for ergonomic task creation with metadata
