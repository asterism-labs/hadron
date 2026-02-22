//! Async scheduling primitives — kernel glue.
//!
//! Re-exports core primitives from `hadron-sched` and adds
//! timer-dependent sleep functions.

pub use hadron_sched::primitives::*;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

/// Sleeps for at least `ticks` timer ticks (1 tick = 1ms at 1kHz).
pub async fn sleep_ticks(ticks: u64) {
    let deadline = crate::time::Time::timer_ticks() + ticks;
    SleepFuture { deadline }.await;
}

/// Sleeps for at least `ms` milliseconds.
pub async fn sleep_ms(ms: u64) {
    sleep_ticks(ms).await;
}

struct SleepFuture {
    deadline: u64,
}

impl Future for SleepFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if crate::time::Time::timer_ticks() >= self.deadline {
            Poll::Ready(())
        } else {
            crate::sched::timer::register_sleep_waker(self.deadline, cx.waker().clone());
            Poll::Pending
        }
    }
}
