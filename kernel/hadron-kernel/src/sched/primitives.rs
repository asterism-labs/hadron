//! Async scheduling primitives.
//!
//! Provides cooperative yielding and timer-based sleeping for kernel tasks.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

/// Cooperatively yields execution to the next ready task.
///
/// Returns `Pending` once (re-queuing via waker), then `Ready` on the
/// next poll. This allows other tasks in the ready queue to run.
pub async fn yield_now() {
    YieldNow(false).await;
}

struct YieldNow(bool);

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Sleeps for at least `ticks` timer ticks (1 tick = 1ms at 1kHz).
pub async fn sleep_ticks(ticks: u64) {
    let deadline = crate::time::timer_ticks() + ticks;
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
        if crate::time::timer_ticks() >= self.deadline {
            Poll::Ready(())
        } else {
            crate::sched::timer::register_sleep_waker(self.deadline, cx.waker().clone());
            Poll::Pending
        }
    }
}

// ---------------------------------------------------------------------------
// join combinator
// ---------------------------------------------------------------------------

/// Polls two futures concurrently, returning both results when both complete.
pub fn join<A: Future, B: Future>(a: A, b: B) -> Join<A, B> {
    Join {
        a,
        b,
        a_result: None,
        b_result: None,
    }
}

/// Future returned by [`join`].
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Join<A: Future, B: Future> {
    a: A,
    b: B,
    a_result: Option<A::Output>,
    b_result: Option<B::Output>,
}

impl<A: Future, B: Future> Future for Join<A, B> {
    type Output = (A::Output, B::Output);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We never move the inner futures after pinning.
        let this = unsafe { self.get_unchecked_mut() };

        if this.a_result.is_none() {
            // SAFETY: `this.a` is structurally pinned.
            let a = unsafe { Pin::new_unchecked(&mut this.a) };
            if let Poll::Ready(val) = a.poll(cx) {
                this.a_result = Some(val);
            }
        }

        if this.b_result.is_none() {
            // SAFETY: `this.b` is structurally pinned.
            let b = unsafe { Pin::new_unchecked(&mut this.b) };
            if let Poll::Ready(val) = b.poll(cx) {
                this.b_result = Some(val);
            }
        }

        if this.a_result.is_some() && this.b_result.is_some() {
            let a = this.a_result.take().unwrap();
            let b = this.b_result.take().unwrap();
            Poll::Ready((a, b))
        } else {
            Poll::Pending
        }
    }
}

// ---------------------------------------------------------------------------
// select combinator
// ---------------------------------------------------------------------------

/// Result of a [`select`]: indicates which future completed first.
pub enum Either<A, B> {
    /// The first future completed first.
    Left(A),
    /// The second future completed first.
    Right(B),
}

/// Polls two futures concurrently, returning the result of whichever completes first.
///
/// The losing future is dropped.
pub fn select<A: Future, B: Future>(a: A, b: B) -> Select<A, B> {
    Select { a, b }
}

/// Future returned by [`select`].
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Select<A: Future, B: Future> {
    a: A,
    b: B,
}

impl<A: Future, B: Future> Future for Select<A, B> {
    type Output = Either<A::Output, B::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We never move the inner futures after pinning.
        let this = unsafe { self.get_unchecked_mut() };

        // SAFETY: `this.a` is structurally pinned.
        let a = unsafe { Pin::new_unchecked(&mut this.a) };
        if let Poll::Ready(val) = a.poll(cx) {
            return Poll::Ready(Either::Left(val));
        }

        // SAFETY: `this.b` is structurally pinned.
        let b = unsafe { Pin::new_unchecked(&mut this.b) };
        if let Poll::Ready(val) = b.poll(cx) {
            return Poll::Ready(Either::Right(val));
        }

        Poll::Pending
    }
}
