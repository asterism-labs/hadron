//! Kernel task scheduler.
//!
//! Provides an async executor that runs kernel tasks as cooperative futures.
//! Tasks yield at `.await` points; a budget-based preemption flag ensures
//! fairness even if a task polls for a long time between awaits.

pub mod executor;
pub mod primitives;
pub mod timer;
mod waker;

pub use executor::Executor;
pub use hadron_core::task::{Priority, TaskMeta};

use core::sync::atomic::{AtomicBool, Ordering};

use hadron_core::task::TaskId;

/// Per-CPU preemption flag (global for now, BSP-only).
static PREEMPT_PENDING: AtomicBool = AtomicBool::new(false);

/// Returns a reference to the global executor.
pub fn executor() -> &'static Executor {
    executor::global()
}

/// Spawns an async kernel task with default (Normal) priority.
pub fn spawn(
    future: impl core::future::Future<Output = ()> + Send + 'static,
) -> TaskId {
    executor().spawn(future)
}

/// Spawns an async kernel task with explicit metadata.
pub fn spawn_with(
    future: impl core::future::Future<Output = ()> + Send + 'static,
    meta: TaskMeta,
) -> TaskId {
    executor().spawn_with_meta(future, meta)
}

/// Spawns a Critical-priority task (interrupt bottom-halves, hardware events).
pub fn spawn_critical(
    name: &'static str,
    future: impl core::future::Future<Output = ()> + Send + 'static,
) -> TaskId {
    executor().spawn_with_meta(
        future,
        TaskMeta::new(name).with_priority(Priority::Critical),
    )
}

/// Spawns a Background-priority task (housekeeping, statistics).
pub fn spawn_background(
    name: &'static str,
    future: impl core::future::Future<Output = ()> + Send + 'static,
) -> TaskId {
    executor().spawn_with_meta(
        future,
        TaskMeta::new(name).with_priority(Priority::Background),
    )
}

/// Sets the preemption-pending flag (called from timer interrupt).
pub fn set_preempt_pending() {
    PREEMPT_PENDING.store(true, Ordering::Release);
}

/// Returns `true` if preemption is pending and clears the flag.
pub(crate) fn preempt_pending() -> bool {
    PREEMPT_PENDING.load(Ordering::Acquire)
}

/// Clears the preemption-pending flag.
pub(crate) fn clear_preempt_pending() {
    PREEMPT_PENDING.store(false, Ordering::Release);
}
