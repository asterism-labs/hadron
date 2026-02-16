//! Async executor for kernel tasks.
//!
//! Provides a cooperative executor that polls `Future<Output = ()>` tasks.
//! Each task is a heap-allocated, pinned, dynamically-dispatched future.
//! The executor runs one per CPU (currently BSP only) and never returns.
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

use hadron_core::sync::{IrqSpinLock, LazyLock};
use hadron_core::task::{Priority, TaskId, TaskMeta};

/// Global executor instance, initialized on first access.
static EXECUTOR: LazyLock<Executor> = LazyLock::new(Executor::new);

/// Returns a reference to the global executor.
pub fn global() -> &'static Executor {
    &EXECUTOR
}

/// A pinned, heap-allocated, dynamically dispatched future.
type TaskFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A stored task: its future plus metadata.
struct TaskEntry {
    future: TaskFuture,
    #[allow(dead_code)] // Phase 7+: used for task debugging, tracing, and affinity routing
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
}

/// The kernel's async task executor.
///
/// Tasks are spawned as `Future<Output = ()> + Send + 'static` and polled
/// cooperatively. A waker-based ready queue ensures only runnable tasks
/// are polled. Tasks are organized into priority tiers.
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
            tasks: IrqSpinLock::new(BTreeMap::new()),
            ready_queues: IrqSpinLock::new(ReadyQueues::new()),
            next_id: AtomicU64::new(0),
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
        id
    }

    /// Main executor loop. Called once per CPU, never returns.
    pub fn run(&self) -> ! {
        loop {
            self.poll_ready_tasks();
            // Nothing ready — halt until next interrupt wakes us.
            #[cfg(target_arch = "x86_64")]
            {
                // SAFETY: IDT and LAPIC are fully configured before executor starts.
                unsafe {
                    hadron_core::arch::x86_64::instructions::interrupts::enable_and_hlt();
                }
                // Interrupt fired — disable interrupts and check for ready tasks.
                hadron_core::arch::x86_64::instructions::interrupts::disable();
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
    /// dropped and interrupts are re-enabled during `future.poll()`. This
    /// allows timer interrupts to fire mid-poll, making the `preempt_pending`
    /// budget check effective.
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
            // Lock dropped here — interrupts re-enabled.

            // Poll with interrupts enabled. Timer CAN fire here.
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

            // Budget check: if timer fired during this batch, yield to main loop.
            // Now actually works because timer can fire during poll above.
            if super::preempt_pending() {
                super::clear_preempt_pending();
                break;
            }
        }
    }
}
