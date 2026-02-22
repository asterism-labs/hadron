//! Kernel task scheduler.
//!
//! Provides per-CPU async executors that run kernel tasks as cooperative
//! futures. Tasks yield at `.await` points; a per-CPU budget-based
//! preemption flag ensures fairness even if a task polls for a long time
//! between awaits.

pub mod block_on;
pub mod executor;
pub mod primitives;
pub mod smp;
pub mod timer;
mod waker;

pub use crate::task::{Priority, TaskMeta};
pub use executor::Executor;

use core::sync::atomic::{AtomicBool, Ordering};

use crate::percpu::{CpuLocal, MAX_CPUS};
use crate::task::TaskId;

/// Per-CPU preemption flag.
static PREEMPT_PENDING: CpuLocal<AtomicBool> =
    CpuLocal::new([const { AtomicBool::new(false) }; MAX_CPUS]);

/// Returns a reference to the current CPU's executor.
pub fn executor() -> &'static Executor {
    executor::global()
}

/// Spawns an async kernel task with default (Normal) priority.
pub fn spawn(future: impl core::future::Future<Output = ()> + Send + 'static) -> TaskId {
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

/// Sets the preemption-pending flag on the current CPU (called from timer interrupt).
pub fn set_preempt_pending() {
    PREEMPT_PENDING.get().store(true, Ordering::Release);
}

/// Returns `true` if preemption is pending on the current CPU.
pub(crate) fn preempt_pending() -> bool {
    PREEMPT_PENDING.get().load(Ordering::Acquire)
}

/// Clears the preemption-pending flag on the current CPU.
pub(crate) fn clear_preempt_pending() {
    PREEMPT_PENDING.get().store(false, Ordering::Release);
}
