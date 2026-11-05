//! Wait queues: wakers waiting for a condition.
//!
//! A waiter registers its waker before it checks the condition a second
//! time, so a wakeup between the check and going to sleep is not lost. A
//! waiter whose condition came true removes its waker, so [`wake_one`]
//! always reaches a waiter that still waits.
//!
//! [`wake_one`]: WaitQueue::wake_one

use alloc::collections::VecDeque;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use super::level::levels;
use super::spin::IrqSpinLock;
use crate::sched;

/// Wakers waiting for something. Wakes are IRQ-safe.
pub(crate) struct WaitQueue {
    wakers: IrqSpinLock<levels::WaitQueue, VecDeque<Waker>>,
}

impl WaitQueue {
    /// A queue with no waiters.
    pub(crate) const fn new() -> Self {
        Self {
            wakers: IrqSpinLock::new(VecDeque::new()),
        }
    }

    /// A future that completes once `condition` returns true. It checks
    /// again each time the queue wakes it.
    pub(crate) fn wait_until<F: FnMut() -> bool>(&self, condition: F) -> WaitUntil<'_, F> {
        WaitUntil {
            queue: self,
            condition,
            registered: None,
        }
    }

    /// Blocks the current thread until `condition` returns true.
    ///
    /// # Panics
    ///
    /// Panics if the thread may not sleep (see [`sched::might_sleep`]).
    #[track_caller]
    pub(crate) fn block_until(&self, condition: impl FnMut() -> bool) {
        sched::block_on(self.wait_until(condition));
    }

    /// Wakes the waiter that registered first.
    pub(crate) fn wake_one(&self) {
        let waker = self.wakers.lock().pop_front();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Wakes every waiter.
    pub(crate) fn wake_all(&self) {
        let wakers = core::mem::take(&mut *self.wakers.lock());
        for waker in wakers {
            waker.wake();
        }
    }

    fn register(&self, waker: &Waker) {
        let mut wakers = self.wakers.lock();
        if !wakers.iter().any(|w| w.will_wake(waker)) {
            wakers.push_back(waker.clone());
        }
    }

    /// Removes the queue's copy of `waker`. The caller still holds
    /// `waker`, so the copy is never the last reference to a thread, and
    /// dropping it under the lock frees nothing.
    fn unregister(&self, waker: &Waker) {
        self.wakers.lock().retain(|w| !w.will_wake(waker));
    }
}

/// The future [`WaitQueue::wait_until`] returns.
pub(crate) struct WaitUntil<'a, F> {
    queue: &'a WaitQueue,
    condition: F,
    /// The waker this future left in the queue, if any.
    registered: Option<Waker>,
}

impl<F: FnMut() -> bool> Future for WaitUntil<'_, F> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if (this.condition)() {
            this.finish();
            return Poll::Ready(());
        }
        this.queue.register(cx.waker());
        this.registered = Some(cx.waker().clone());
        // A wake that came before the waker was in the queue would be lost
        // without this second check.
        if (this.condition)() {
            this.finish();
            return Poll::Ready(());
        }
        Poll::Pending
    }
}

// Nothing in the future is pinned structurally: `condition` is only
// called through `&mut`.
impl<F> Unpin for WaitUntil<'_, F> {}

impl<F> WaitUntil<'_, F> {
    fn finish(&mut self) {
        if let Some(waker) = self.registered.take() {
            self.queue.unregister(&waker);
        }
    }
}

impl<F> Drop for WaitUntil<'_, F> {
    fn drop(&mut self) {
        self.finish();
    }
}
