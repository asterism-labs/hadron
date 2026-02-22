//! Async executor for kernel tasks.
//!
//! Provides a cooperative executor that polls `Future<Output = ()>` tasks.
//! Each task is a heap-allocated, pinned, dynamically-dispatched future.
//! Each CPU runs its own executor instance (accessed via [`global`] /
//! [`for_cpu`]). Tasks stay on their spawning CPU unless migrated by work
//! stealing (Phase 12.6).
//!
//! Tasks are organized into three strict priority tiers: Critical, Normal,
//! and Background. The executor always drains higher-priority tiers first.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU64, Ordering};
use core::task::{Context, Poll};

use crate::id::CpuId;
use crate::percpu::{CpuLocal, MAX_CPUS};
use crate::sync::{IrqSpinLock, LazyLock};
use crate::task::{Priority, TaskId, TaskMeta};

pub use hadron_core::sched::ReadyQueues;

/// Per-CPU executor instances, initialized on first access.
static EXECUTORS: CpuLocal<LazyLock<Executor>> =
    CpuLocal::new([const { LazyLock::new(Executor::new as fn() -> Executor) }; MAX_CPUS]);

/// Returns a reference to the current CPU's executor.
pub fn global() -> &'static Executor {
    EXECUTORS.get()
}

/// Returns a reference to a specific CPU's executor.
///
/// Used by the waker to push tasks back to their originating CPU's queue.
pub fn for_cpu(cpu_id: CpuId) -> &'static Executor {
    EXECUTORS.get_for(cpu_id)
}

/// A pinned, heap-allocated, dynamically dispatched future.
type TaskFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A stored task: its future plus metadata.
pub(crate) struct TaskEntry {
    future: TaskFuture,
    #[allow(dead_code, reason = "reserved for Phase 7+ task debugging")]
    meta: TaskMeta,
}

/// The kernel's async task executor.
///
/// Tasks are spawned as `Future<Output = ()> + Send + 'static` and polled
/// cooperatively. A waker-based ready queue ensures only runnable tasks
/// are polled. Tasks are organized into priority tiers.
///
/// Each CPU has its own executor instance. Tasks are spawned on the current
/// CPU's executor and stay there unless migrated by work stealing.
pub struct Executor {
    /// Task storage: maps TaskId -> task entry (future + metadata).
    tasks: IrqSpinLock<BTreeMap<TaskId, TaskEntry>>,
    /// Priority-aware ready queues.
    pub(crate) ready_queues: IrqSpinLock<ReadyQueues>,
    /// Next task ID counter.
    next_id: AtomicU64,
}

impl Executor {
    /// Creates a new executor with no tasks.
    pub fn new() -> Self {
        Self {
            tasks: IrqSpinLock::leveled("Executor.tasks", 14, BTreeMap::new()),
            ready_queues: IrqSpinLock::leveled("Executor.ready_queues", 13, ReadyQueues::new()),
            next_id: AtomicU64::new(0),
        }
    }

    /// Attempts to steal one task from this executor for work stealing.
    ///
    /// Steals from the back of the ready queue (to preserve locality for
    /// the victim's hot tasks) and removes the corresponding `TaskEntry`
    /// from the task map. Uses `try_lock` to avoid blocking the victim.
    ///
    /// Returns `None` if:
    /// - The ready queues or task map can't be locked (contention)
    /// - No stealable tasks exist (Critical tasks are never stolen)
    /// - The task is currently being polled (entry not in task map)
    pub(crate) fn steal_task(&self) -> Option<(TaskId, Priority, TaskEntry)> {
        let mut rq = self.ready_queues.try_lock()?;
        let (priority, id) = rq.steal_one()?;
        // Hold ready_queues lock while checking tasks to prevent the
        // stolen task ID from being lost. try_lock avoids deadlock with
        // spawn (which takes tasks then ready_queues — opposite order).
        let entry = self
            .tasks
            .try_lock()
            .and_then(|mut tasks| tasks.remove(&id));
        match entry {
            Some(entry) => Some((id, priority, entry)),
            None => {
                // Task is being polled or tasks lock contended — put it back.
                rq.push(priority, id);
                None
            }
        }
    }

    /// Spawns a new async task with default metadata (Normal priority).
    pub fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) -> TaskId {
        self.spawn_with_meta(future, TaskMeta::default())
    }

    /// Spawns a new async task with explicit metadata.
    pub fn spawn_with_meta(
        &self,
        future: impl Future<Output = ()> + Send + 'static,
        meta: TaskMeta,
    ) -> TaskId {
        let id = TaskId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let priority = meta.priority;
        // Allocate the boxed future BEFORE acquiring the tasks lock to avoid
        // a level ordering violation (tasks=14 → HEAP=1 is descending).
        let boxed_future = Box::pin(future);
        self.tasks.lock().insert(
            id,
            TaskEntry {
                future: boxed_future,
                meta,
            },
        );
        self.ready_queues.lock().push(priority, id);
        crate::ktrace_subsys!(sched, "spawned task id={} priority={:?}", id.0, priority);
        id
    }

    /// Main executor loop. Called once per CPU, never returns.
    pub fn run(&self) -> ! {
        loop {
            // Clear the stale preempt_pending flag left by the timer
            // interrupt that woke us from HLT.
            super::clear_preempt_pending();

            self.poll_ready_tasks();

            // Re-poll if preempt_pending broke poll_ready_tasks early and
            // tasks remain. Checking BEFORE try_steal prevents the bouncing
            // livelock: without this, try_steal scans all CPUs (expensive,
            // opens a window for others to steal our stranded tasks), causing
            // tasks to bounce between CPUs without real forward progress.
            if self.ready_queues.lock().has_ready() {
                continue;
            }

            // No local work — try to steal from another CPU before halting.
            if let Some((id, priority, entry)) = super::smp::try_steal() {
                // Insert the stolen task into our local task map and
                // ready queue. When polled, the new waker will encode
                // this CPU's ID, effectively migrating the task.
                self.tasks.lock().insert(id, entry);
                self.ready_queues.lock().push(priority, id);
                continue;
            }

            // Nothing ready — halt until next interrupt wakes us.
            #[cfg(target_arch = "x86_64")]
            {
                // SAFETY: IDT and LAPIC are fully configured before executor starts.
                unsafe {
                    crate::arch::x86_64::instructions::interrupts::enable_and_hlt();
                }
                // Interrupt fired — disable interrupts and check for ready tasks.
                crate::arch::x86_64::instructions::interrupts::disable();
            }
            #[cfg(target_arch = "aarch64")]
            {
                // SAFETY: exception vectors are configured before executor starts.
                unsafe {
                    core::arch::asm!("wfi", options(nomem, nostack, preserves_flags));
                }
            }
        }
    }

    /// Polls ready tasks until the queue is empty or a ring-3 timer
    /// preemption sets `preempt_pending`.
    ///
    /// The task is removed from `self.tasks` before polling so the lock is
    /// dropped during `future.poll()`. Interrupts remain disabled (IF=0)
    /// throughout because `IrqSpinLock` save/restore preserves the IF
    /// state from the caller (`run()` disables after HLT).
    ///
    /// The `preempt_pending` yield point returns control to `run()` after
    /// a ring-3 timer trap fires inside a `process_task` poll. The caller
    /// re-polls if tasks remain (see the `has_ready` guard in `run()`).
    ///
    /// If a waker fires while the task is out for polling, the task ID is
    /// pushed into the ready queue again. The next iteration will call
    /// `tasks.remove(&id)` and get `None` (the task is still being polled),
    /// which is harmless — the task will be found on a subsequent iteration
    /// after it is re-inserted.
    fn poll_ready_tasks(&self) {
        loop {
            let (priority, id) = match self.ready_queues.lock().pop() {
                Some(pair) => pair,
                None => break,
            };

            let waker = super::waker::task_waker(id, priority);
            let mut cx = Context::from_waker(&waker);

            // Brief lock: take future out of storage.
            let entry = {
                let mut tasks = self.tasks.lock();
                tasks.remove(&id)
            };
            // Lock dropped here (IF stays 0 — IrqSpinLock restores saved flags).

            // Poll the future. Ring-0 interrupts are disabled (IF=0). A
            // ring-3 timer trap inside process_task will longjmp back,
            // causing the future to yield and re-queue itself at the back.
            if let Some(mut entry) = entry {
                match entry.future.as_mut().poll(&mut cx) {
                    Poll::Ready(()) => {
                        // Task complete — don't store it back.
                    }
                    Poll::Pending => {
                        // Brief lock: put future back.
                        self.tasks.lock().insert(id, entry);
                    }
                }
            }

            // Yield point: if a ring-3 timer preemption set the flag during
            // this batch, return to the main loop. The caller re-polls if
            // there are remaining tasks (preventing the livelock where
            // stranded tasks get stolen endlessly without being polled).
            if super::preempt_pending() {
                super::clear_preempt_pending();
                break;
            }
        }
    }
}
