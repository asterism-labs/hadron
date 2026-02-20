//! Heap-backed wait queue with unbounded capacity.

extern crate alloc;

use alloc::collections::VecDeque;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use super::IrqSpinLock;

/// Heap-backed wait queue with unbounded capacity.
///
/// Unlike the frame-layer [`super::WaitQueue`] (fixed 32 slots), this uses
/// `VecDeque<Waker>` and can hold any number of waiters. For service-layer
/// primitives (channels, barriers) where many tasks may wait.
///
/// Uses `VecDeque` so that `wake_one()` is O(1) FIFO (pop_front) rather
/// than O(n) (Vec::swap_remove(0) shifts elements or breaks FIFO order).
pub struct HeapWaitQueue {
    waiters: IrqSpinLock<VecDeque<Waker>>,
}

impl HeapWaitQueue {
    /// Creates an empty heap-backed wait queue.
    pub const fn new() -> Self {
        Self {
            waiters: IrqSpinLock::new(VecDeque::new()),
        }
    }

    /// Returns a future that completes when this queue is woken.
    pub fn wait(&self) -> HeapWaitFuture<'_> {
        HeapWaitFuture {
            queue: self,
            registered: false,
        }
    }

    /// Registers a waker without creating a future.
    pub fn register_waker(&self, waker: &Waker) {
        self.waiters.lock().push_back(waker.clone());
    }

    /// Wakes one waiting task (FIFO order, O(1)).
    pub fn wake_one(&self) {
        let waker = {
            let mut waiters = self.waiters.lock();
            waiters.pop_front()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }

    /// Wakes all waiting tasks.
    pub fn wake_all(&self) {
        let drained: VecDeque<Waker> = {
            let mut waiters = self.waiters.lock();
            core::mem::take(&mut *waiters)
        };
        for w in drained {
            w.wake();
        }
    }
}

/// Future returned by [`HeapWaitQueue::wait`].
pub struct HeapWaitFuture<'a> {
    queue: &'a HeapWaitQueue,
    registered: bool,
}

impl Future for HeapWaitFuture<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.registered {
            Poll::Ready(())
        } else {
            self.registered = true;
            self.queue.waiters.lock().push_back(cx.waker().clone());
            Poll::Pending
        }
    }
}
