//! Heap-backed wait queue with unbounded capacity.

extern crate alloc;

use alloc::collections::VecDeque;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use super::backend::{CoreBackend, IrqBackend};
use super::irq_spinlock::{IrqSpinLock, IrqSpinLockInner};

// ─── Type aliases ─────────────────────────────────────────────────────

/// Heap-backed wait queue with unbounded capacity.
///
/// Unlike the frame-layer [`super::WaitQueue`] (fixed 32 slots), this uses
/// `VecDeque<Waker>` and can hold any number of waiters. For service-layer
/// primitives (channels, barriers) where many tasks may wait.
///
/// Uses `VecDeque` so that `wake_one()` is O(1) FIFO (pop_front) rather
/// than O(n) (Vec::swap_remove(0) shifts elements or breaks FIFO order).
pub type HeapWaitQueue = HeapWaitQueueInner<CoreBackend>;

/// Future returned by [`HeapWaitQueue::wait`].
pub type HeapWaitFuture<'a> = HeapWaitFutureInner<'a, CoreBackend>;

// ─── Generic inner type ───────────────────────────────────────────────

/// Backend-generic heap-backed wait queue.
pub struct HeapWaitQueueInner<B: IrqBackend> {
    waiters: IrqSpinLockInner<VecDeque<Waker>, B>,
}

// ─── Const constructor (CoreBackend only) ─────────────────────────────

impl HeapWaitQueue {
    /// Creates an empty heap-backed wait queue.
    pub const fn new() -> Self {
        Self {
            waiters: IrqSpinLock::new(VecDeque::new()),
        }
    }
}

// ─── Generic non-const constructor ────────────────────────────────────

impl<B: IrqBackend> HeapWaitQueueInner<B> {
    /// Creates an empty heap-backed wait queue using backend factory functions.
    pub fn new_with_backend() -> Self {
        Self {
            waiters: IrqSpinLockInner::new_with_backend(VecDeque::new()),
        }
    }
}

// ─── Algorithm (generic over B) ───────────────────────────────────────

impl<B: IrqBackend> HeapWaitQueueInner<B> {
    /// Returns a future that completes when this queue is woken.
    pub fn wait(&self) -> HeapWaitFutureInner<'_, B> {
        HeapWaitFutureInner {
            queue: self,
            registered: false,
        }
    }

    /// Registers a waker without creating a future.
    ///
    /// If an identical waker (same task, same CPU) is already queued, the
    /// existing entry is kept and no duplicate is added. This prevents stale
    /// waker accumulation when a task is re-polled on the same CPU before a
    /// prior wakeup fires.
    pub fn register_waker(&self, waker: &Waker) {
        let mut waiters = self.waiters.lock();
        for w in waiters.iter() {
            if w.will_wake(waker) {
                return; // already registered
            }
        }
        waiters.push_back(waker.clone());
    }

    /// Returns `true` if there are registered waiters (diagnostic use only).
    #[cfg(hadron_lock_debug)]
    pub fn has_waiters(&self) -> bool {
        !self.waiters.lock().is_empty()
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

// ─── Wait future ──────────────────────────────────────────────────────

/// Future returned by [`HeapWaitQueueInner::wait`].
pub struct HeapWaitFutureInner<'a, B: IrqBackend> {
    queue: &'a HeapWaitQueueInner<B>,
    registered: bool,
}

impl<B: IrqBackend> Future for HeapWaitFutureInner<'_, B> {
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

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Verify that wake operations on an empty heap wait queue do not panic.
    #[kani::proof]
    fn heap_waitqueue_empty_ops() {
        let wq = HeapWaitQueue::new();
        // Operations on an empty queue must be no-ops, never panic.
        wq.wake_one();
        wq.wake_all();
    }
}
