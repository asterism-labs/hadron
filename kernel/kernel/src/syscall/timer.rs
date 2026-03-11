//! Timer syscall handlers: create, set, cancel.

extern crate alloc;

use alloc::sync::Arc;

use hadron_objects::handle::{HandleEntry, HandleValue, Rights};
use hadron_objects::object::KernelObject;
use hadron_objects::timer::Timer;
use hadron_syscall::*;

use super::with_handle_table;

/// `SYS_TIMER_CREATE()` — create a new unarmed timer.
///
/// Returns a handle to the new Timer, or a negative error code.
pub fn sys_timer_create() -> isize {
    let timer = Timer::new();
    let entry = HandleEntry::new(timer as Arc<dyn KernelObject>, Rights::TIMER_DEFAULT);

    with_handle_table(|table| match table.insert(entry) {
        Ok(hv) => hv.raw() as isize,
        Err(_) => -EMFILE,
    })
}

/// `SYS_TIMER_SET(fd, deadline_ns, slack_ns)` — arm a timer with a deadline.
///
/// When the deadline elapses, a spawned future on the per-CPU executor calls
/// `Timer::trigger()`, which asserts `SIGNAL_0` and notifies observers.
pub fn sys_timer_set(fd: usize, deadline_ns: usize, slack_ns: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);

    let obj = with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::WRITE) {
            Ok(e) => e,
            Err(_) => return Err(-EBADF),
        };

        if entry.object().as_any().downcast_ref::<Timer>().is_none() {
            return Err(-EINVAL);
        }

        Ok(Arc::clone(entry.object()))
    });

    let obj = match obj {
        Ok(o) => o,
        Err(e) => return e,
    };

    // Arm the timer (clears any prior SIGNAL_0).
    let timer = obj
        .as_any()
        .downcast_ref::<Timer>()
        .expect("validated as Timer");
    timer.set(deadline_ns as u64, slack_ns as u64);

    // If the deadline has already passed, trigger immediately.
    if crate::time::nanos_since_boot() >= deadline_ns as u64 {
        timer.trigger();
        return 0;
    }

    // Spawn a lightweight future to trigger the timer when the deadline elapses.
    // If the timer is re-set or canceled before then, trigger() becomes a no-op
    // because the deadline field will have changed.
    hadron_sched::spawn(timer_trigger_task(obj, deadline_ns as u64));

    0
}

/// A future that sleeps until a deadline, then triggers the timer.
///
/// Uses `SleepUntil`-equivalent logic: registers a waker with the kernel
/// timer subsystem and waits for the deadline.
async fn timer_trigger_task(obj: Arc<dyn KernelObject>, deadline_ns: u64) {
    // Wait until the deadline.
    TimerSleep::new(deadline_ns).await;

    // Fire the timer if it's still armed at this deadline.
    let timer = obj
        .as_any()
        .downcast_ref::<Timer>()
        .expect("timer_trigger_task: object is Timer");

    // Only trigger if the timer is still armed for this specific deadline.
    // If the user re-armed or canceled, deadline() will be different.
    if timer.deadline() == Some(deadline_ns) {
        timer.trigger();
    }
}

/// A future that resolves when the monotonic clock reaches a deadline.
///
/// Equivalent to the process.rs `SleepUntil` but usable outside `process_task`.
struct TimerSleep {
    deadline_ns: u64,
}

impl TimerSleep {
    fn new(deadline_ns: u64) -> Self {
        Self { deadline_ns }
    }
}

impl core::future::Future for TimerSleep {
    type Output = ();

    fn poll(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<()> {
        if crate::time::nanos_since_boot() >= self.deadline_ns {
            core::task::Poll::Ready(())
        } else {
            hadron_sched::timer::register_sleep_waker(self.deadline_ns, cx.waker().clone());
            core::task::Poll::Pending
        }
    }
}

/// `SYS_TIMER_CANCEL(fd)` — cancel a pending timer.
pub fn sys_timer_cancel(fd: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);

    with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::WRITE) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let timer = match entry.object().as_any().downcast_ref::<Timer>() {
            Some(t) => t,
            None => return -EINVAL,
        };

        timer.cancel();
        0
    })
}
