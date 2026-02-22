//! Wait queue for interrupt-driven wakeups.
//!
//! [`WaitQueue`] stores [`Waker`]s from async tasks that are waiting for
//! an event. Interrupt handlers or other kernel code call [`wake_one`] or
//! [`wake_all`] to resume those tasks.
//!
//! Uses a fixed-capacity [`ArrayVec`] to avoid requiring a heap allocator
//! in hadron-core.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use planck_noalloc::vec::ArrayVec;

use crate::sync::IrqSpinLock;

/// Maximum number of waiters per queue.
const MAX_WAITERS: usize = 32;

/// A queue of [`Waker`]s waiting for an event.
///
/// Tasks call [`wait`](WaitQueue::wait) to obtain a future that completes
/// when the queue is woken. Interrupt handlers call [`wake_one`](WaitQueue::wake_one)
/// or [`wake_all`](WaitQueue::wake_all) to resume waiting tasks.
pub struct WaitQueue {
    waiters: IrqSpinLock<ArrayVec<Waker, MAX_WAITERS>>,
}

impl WaitQueue {
    /// Creates an empty wait queue.
    pub const fn new() -> Self {
        Self {
            waiters: IrqSpinLock::new(ArrayVec::new()),
        }
    }

    /// Returns a future that completes when this queue is woken.
    pub fn wait(&self) -> WaitFuture<'_> {
        WaitFuture {
            queue: self,
            registered: false,
        }
    }

    /// Registers a waker without creating a future.
    ///
    /// Used by [`Mutex`](crate::sync::Mutex) to register interest before
    /// retrying acquisition. Returns `true` if the waker was registered,
    /// `false` if the queue is full.
    pub fn register_waker(&self, waker: &Waker) -> bool {
        let mut waiters = self.waiters.lock();
        if waiters.len() < MAX_WAITERS {
            waiters.push(waker.clone());
            true
        } else {
            false
        }
    }

    /// Wakes one waiting task (FIFO order).
    pub fn wake_one(&self) {
        let mut waiters = self.waiters.lock();
        if !waiters.is_empty() {
            let waker = waiters.swap_remove(0);
            drop(waiters);
            waker.wake();
        }
    }

    /// Wakes all waiting tasks.
    pub fn wake_all(&self) {
        let mut waiters = self.waiters.lock();
        // Drain all wakers, then wake them outside the lock.
        let mut temp = ArrayVec::<Waker, MAX_WAITERS>::new();
        while let Some(w) = waiters.pop() {
            temp.push(w);
        }
        drop(waiters);
        while let Some(w) = temp.pop() {
            w.wake();
        }
    }
}

/// Future returned by [`WaitQueue::wait`].
pub struct WaitFuture<'a> {
    queue: &'a WaitQueue,
    registered: bool,
}

impl Future for WaitFuture<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.registered {
            // We were woken — complete.
            Poll::Ready(())
        } else {
            self.registered = true;
            let mut waiters = self.queue.waiters.lock();
            if waiters.len() < MAX_WAITERS {
                waiters.push(cx.waker().clone());
            }
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_waker::{counting_waker, noop_waker};
    use std::sync::atomic::Ordering;

    #[test]
    fn register_waker_succeeds() {
        let wq = WaitQueue::new();
        let waker = noop_waker();
        assert!(wq.register_waker(&waker));
    }

    #[test]
    fn register_waker_full() {
        let wq = WaitQueue::new();
        let waker = noop_waker();
        for _ in 0..MAX_WAITERS {
            assert!(wq.register_waker(&waker));
        }
        // 33rd waker should fail.
        assert!(!wq.register_waker(&waker));
    }

    #[test]
    fn wake_one_fifo() {
        let wq = WaitQueue::new();
        let (w1, c1) = counting_waker();
        let (w2, c2) = counting_waker();
        wq.register_waker(&w1);
        wq.register_waker(&w2);

        wq.wake_one();
        assert!(c1.load(Ordering::SeqCst) > 0, "first waker should be woken");
        assert_eq!(
            c2.load(Ordering::SeqCst),
            0,
            "second waker should not be woken"
        );
    }

    #[test]
    fn wake_all_wakes_everyone() {
        let wq = WaitQueue::new();
        let (w1, c1) = counting_waker();
        let (w2, c2) = counting_waker();
        let (w3, c3) = counting_waker();
        wq.register_waker(&w1);
        wq.register_waker(&w2);
        wq.register_waker(&w3);

        wq.wake_all();
        assert!(c1.load(Ordering::SeqCst) > 0);
        assert!(c2.load(Ordering::SeqCst) > 0);
        assert!(c3.load(Ordering::SeqCst) > 0);
    }

    #[test]
    fn wake_one_empty_no_panic() {
        let wq = WaitQueue::new();
        wq.wake_one(); // should not panic
    }

    #[test]
    fn wake_all_empty_no_panic() {
        let wq = WaitQueue::new();
        wq.wake_all(); // should not panic
    }

    #[test]
    fn wait_future_pending_then_ready() {
        let wq = WaitQueue::new();
        let waker = noop_waker();
        let mut cx = core::task::Context::from_waker(&waker);
        let mut fut = wq.wait();

        // First poll should return Pending and register the waker.
        let result = Pin::new(&mut fut).poll(&mut cx);
        assert!(matches!(result, Poll::Pending));

        // Second poll should return Ready (the `registered` flag is set).
        let result = Pin::new(&mut fut).poll(&mut cx);
        assert!(matches!(result, Poll::Ready(())));
    }
}
