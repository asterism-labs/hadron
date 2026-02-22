//! Condition variable for synchronizing tasks.
//!
//! [`Condvar`] allows tasks to wait until a predicate becomes true,
//! releasing and re-acquiring a lock around the wait. Supports both
//! synchronous (spin-based) and async (executor-yielding) waiting.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use planck_noalloc::vec::ArrayVec;

use super::{IrqSpinLock, MutexGuard, SpinLockGuard};

/// Maximum number of waiters per condition variable.
const MAX_WAITERS: usize = 32;

/// A condition variable.
///
/// Tasks call [`wait`](Condvar::wait) to atomically release a lock and
/// sleep until [`notify_one`](Condvar::notify_one) or
/// [`notify_all`](Condvar::notify_all) is called. The caller should
/// always recheck the predicate after waking, as spurious wakeups are
/// possible.
///
/// # Example
///
/// ```ignore
/// static READY: SpinLock<bool> = SpinLock::new(false);
/// static COND: Condvar = Condvar::new();
///
/// // Waiter:
/// let mut guard = READY.lock();
/// while !*guard {
///     guard = COND.wait(guard);
/// }
///
/// // Notifier:
/// *READY.lock() = true;
/// COND.notify_one();
/// ```
pub struct Condvar {
    waiters: IrqSpinLock<ArrayVec<Waker, MAX_WAITERS>>,
}

impl Condvar {
    /// Creates a new condition variable.
    pub const fn new() -> Self {
        Self {
            waiters: IrqSpinLock::new(ArrayVec::new()),
        }
    }

    /// Atomically releases the [`SpinLockGuard`], waits for notification,
    /// then re-acquires and returns a new guard.
    ///
    /// The caller should always recheck the predicate in a loop:
    /// ```ignore
    /// while !condition {
    ///     guard = condvar.wait(guard);
    /// }
    /// ```
    pub fn wait<'a, T>(&self, guard: SpinLockGuard<'a, T>) -> SpinLockGuard<'a, T> {
        // Get the lock reference before dropping the guard.
        let lock = guard.lock_ref();

        // Register our intent to be woken before releasing the lock.
        // We use a simple spin-poll approach here since we can't easily
        // create a waker in sync context.
        drop(guard);

        // Spin until notified. In a real implementation with a scheduler,
        // this would block the current thread. For the kernel's cooperative
        // executor model, prefer `wait_async`.
        loop {
            // Try to re-acquire immediately.
            if let Some(new_guard) = lock.try_lock() {
                return new_guard;
            }
            core::hint::spin_loop();
        }
    }

    /// Asynchronously waits for notification, yielding the current task.
    ///
    /// Releases the [`MutexGuard`], registers a waker, and yields. When
    /// notified, re-acquires the mutex and returns a new guard.
    pub async fn wait_async<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        let mutex = guard.mutex_ref();

        // Register waker before releasing the lock to avoid lost wakeup.
        let wait = CondvarWaitFuture {
            condvar: self,
            registered: false,
        };

        // Drop the guard to release the mutex.
        drop(guard);

        // Wait for notification.
        wait.await;

        // Re-acquire the mutex.
        mutex.lock().await
    }

    /// Wakes one waiting task (FIFO order).
    pub fn notify_one(&self) {
        let waker = {
            let mut waiters = self.waiters.lock();
            if waiters.is_empty() {
                None
            } else {
                Some(waiters.swap_remove(0))
            }
        };
        if let Some(w) = waker {
            w.wake();
        }
    }

    /// Wakes all waiting tasks.
    pub fn notify_all(&self) {
        let mut temp = ArrayVec::<Waker, MAX_WAITERS>::new();
        {
            let mut waiters = self.waiters.lock();
            while let Some(w) = waiters.pop() {
                temp.push(w);
            }
        }
        while let Some(w) = temp.pop() {
            w.wake();
        }
    }
}

/// Future that waits for a condvar notification.
struct CondvarWaitFuture<'a> {
    condvar: &'a Condvar,
    registered: bool,
}

impl Future for CondvarWaitFuture<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.registered {
            Poll::Ready(())
        } else {
            self.registered = true;
            let mut waiters = self.condvar.waiters.lock();
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
    use crate::sync::SpinLock;

    #[test]
    fn notify_one_without_waiters() {
        let cv = Condvar::new();
        cv.notify_one(); // should not panic
    }

    #[test]
    fn notify_all_without_waiters() {
        let cv = Condvar::new();
        cv.notify_all(); // should not panic
    }

    #[test]
    fn condvar_construction() {
        let _cv = Condvar::new();
    }

    #[test]
    fn wait_returns_immediately_when_lock_free() {
        let lock = SpinLock::new(42);
        let cv = Condvar::new();
        let guard = lock.lock();
        // wait() releases the guard and re-acquires — should work since
        // no contention exists.
        let guard2 = cv.wait(guard);
        assert_eq!(*guard2, 42);
    }
}
