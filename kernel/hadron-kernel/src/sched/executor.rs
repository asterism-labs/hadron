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
use alloc::collections::VecDeque;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU64, Ordering};
use core::task::{Context, Poll};

use crate::percpu::{CpuLocal, MAX_CPUS};
use crate::sync::{IrqSpinLock, LazyLock};
use crate::task::{Priority, TaskId, TaskMeta};

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
pub fn for_cpu(cpu_id: u32) -> &'static Executor {
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

/// Priority-aware ready queues.
///
/// Maintains one FIFO queue per priority tier. Pops from the highest
/// priority (lowest ordinal) non-empty queue first.
pub(crate) struct ReadyQueues {
    queues: [VecDeque<TaskId>; Priority::COUNT],
    /// Counter for background starvation prevention.
    /// Incremented each time a Normal task is popped while Background tasks wait.
    normal_streak: u64,
}

/// How many consecutive Normal polls before forcing one Background poll.
const BACKGROUND_STARVATION_LIMIT: u64 = 100;

impl ReadyQueues {
    fn new() -> Self {
        Self {
            queues: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
            normal_streak: 0,
        }
    }

    /// Pushes a task into the queue for the given priority.
    pub(crate) fn push(&mut self, priority: Priority, id: TaskId) {
        self.queues[priority as usize].push_back(id);
    }

    /// Pops the highest-priority ready task.
    ///
    /// Always drains Critical first. Between Normal and Background,
    /// applies starvation prevention: if Normal has run for
    /// `BACKGROUND_STARVATION_LIMIT` consecutive pops and Background
    /// has tasks, pop one Background task instead.
    fn pop(&mut self) -> Option<(Priority, TaskId)> {
        // Critical always first.
        if let Some(id) = self.queues[Priority::Critical as usize].pop_front() {
            self.normal_streak = 0;
            return Some((Priority::Critical, id));
        }

        // Starvation prevention: if Normal has been running too long
        // and Background has work, give Background a turn.
        let has_background = !self.queues[Priority::Background as usize].is_empty();
        let has_normal = !self.queues[Priority::Normal as usize].is_empty();

        if has_normal && has_background && self.normal_streak >= BACKGROUND_STARVATION_LIMIT {
            self.normal_streak = 0;
            if let Some(id) = self.queues[Priority::Background as usize].pop_front() {
                return Some((Priority::Background, id));
            }
        }

        // Normal next.
        if let Some(id) = self.queues[Priority::Normal as usize].pop_front() {
            if has_background {
                self.normal_streak += 1;
            } else {
                self.normal_streak = 0;
            }
            return Some((Priority::Normal, id));
        }

        // Background last.
        self.normal_streak = 0;
        self.queues[Priority::Background as usize]
            .pop_front()
            .map(|id| (Priority::Background, id))
    }

    /// Steals one task from the back of the queue for work stealing.
    ///
    /// Returns a Normal or Background task (never Critical). Steals from
    /// the back to preserve locality — the victim keeps its hot (front)
    /// tasks while the thief gets the coldest (most recently enqueued) one.
    pub(crate) fn steal_one(&mut self) -> Option<(Priority, TaskId)> {
        // Prefer stealing Normal over Background.
        if let Some(id) = self.queues[Priority::Normal as usize].pop_back() {
            return Some((Priority::Normal, id));
        }
        if let Some(id) = self.queues[Priority::Background as usize].pop_back() {
            return Some((Priority::Background, id));
        }
        None
    }
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
            tasks: IrqSpinLock::leveled("Executor.tasks", 13, BTreeMap::new()),
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
        self.tasks.lock().insert(
            id,
            TaskEntry {
                future: Box::pin(future),
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
            // Clear preempt_pending before polling. The timer interrupt that
            // woke us from HLT calls set_preempt_pending(), but since we run
            // poll_ready_tasks() with IF=0, that stale flag would cause the
            // batch to break after just one task — stranding newly-queued
            // tasks (e.g. shell woken by exit_notify) until the next timer.
            super::clear_preempt_pending();

            self.poll_ready_tasks();

            // Try to steal work from another CPU before halting.
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

    /// Polls all tasks currently in the ready queues, highest priority first.
    ///
    /// The task is removed from `self.tasks` before polling so the lock is
    /// dropped during `future.poll()`. Note: interrupts remain disabled
    /// (IF=0) throughout the batch because `IrqSpinLock` save/restore
    /// preserves the IF state from the caller (`run()` disables interrupts
    /// after HLT). The `preempt_pending` budget check is only effective
    /// when a ring-3 timer trap fires during a `process_task` poll (which
    /// re-enters the executor via longjmp with `preempt_pending` set).
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

            // Poll the future. Interrupts are disabled (IF=0) so no timer
            // can fire here. However, a ring-3 timer trap inside
            // process_task will longjmp back with preempt_pending set.
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

            // Budget check: if a ring-3 timer preemption set the flag during
            // this batch, yield to the main loop so we re-check for steals/HLT.
            if super::preempt_pending() {
                super::clear_preempt_pending();
                break;
            }
        }
    }
}
