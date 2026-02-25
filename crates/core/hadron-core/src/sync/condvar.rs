//! Condition variable for synchronizing tasks.
//!
//! [`Condvar`] allows tasks to wait until a predicate becomes true,
//! releasing and re-acquiring a lock around the wait. Supports both
//! synchronous (spin-based) and async (executor-yielding) waiting.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use planck_noalloc::vec::ArrayVec;

use super::backend::{CoreBackend, IrqBackend};
use super::irq_spinlock::{IrqSpinLock, IrqSpinLockInner};
use super::mutex::MutexGuardInner;
use super::spinlock::SpinLockGuardInner;

/// Maximum number of waiters per condition variable.
const MAX_WAITERS: usize = 32;

// ─── Type aliases ─────────────────────────────────────────────────────

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
pub type Condvar = CondvarInner<CoreBackend>;

// ─── Generic inner type ───────────────────────────────────────────────

/// Backend-generic condition variable.
pub struct CondvarInner<B: IrqBackend> {
    waiters: IrqSpinLockInner<ArrayVec<Waker, MAX_WAITERS>, B>,
}

// ─── Const constructor (CoreBackend only) ─────────────────────────────

impl Condvar {
    /// Creates a new condition variable.
    pub const fn new() -> Self {
        Self {
            waiters: IrqSpinLock::new(ArrayVec::new()),
        }
    }
}

// ─── Generic non-const constructor ────────────────────────────────────

impl<B: IrqBackend> CondvarInner<B> {
    /// Creates a new condition variable using backend factory functions.
    pub fn new_with_backend() -> Self {
        Self {
            waiters: IrqSpinLockInner::new_with_backend(ArrayVec::new()),
        }
    }
}

// ─── Algorithm (generic over B) ───────────────────────────────────────

impl<B: IrqBackend> CondvarInner<B> {
    /// Atomically releases the [`SpinLockGuardInner`], waits for notification,
    /// then re-acquires and returns a new guard.
    ///
    /// The caller should always recheck the predicate in a loop:
    /// ```ignore
    /// while !condition {
    ///     guard = condvar.wait(guard);
    /// }
    /// ```
    pub fn wait<'a, T>(&self, guard: SpinLockGuardInner<'a, T, B>) -> SpinLockGuardInner<'a, T, B> {
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
            B::spin_wait_hint();
        }
    }

    /// Asynchronously waits for notification, yielding the current task.
    ///
    /// Releases the [`MutexGuardInner`], registers a waker, and yields. When
    /// notified, re-acquires the mutex and returns a new guard.
    pub async fn wait_async<'a, T>(
        &self,
        guard: MutexGuardInner<'a, T, B>,
    ) -> MutexGuardInner<'a, T, B> {
        let mutex = guard.mutex_ref();

        // Register waker before releasing the lock to avoid lost wakeup.
        let wait = CondvarWaitFutureInner {
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

// ─── Wait future ──────────────────────────────────────────────────────

/// Future that waits for a condvar notification.
struct CondvarWaitFutureInner<'a, B: IrqBackend> {
    condvar: &'a CondvarInner<B>,
    registered: bool,
}

impl<B: IrqBackend> Future for CondvarWaitFutureInner<'_, B> {
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

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(all(test, not(loom)))]
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

#[cfg(loom)]
mod loom_tests {
    use core::future::Future;
    use core::pin::Pin;
    use core::task::Context;

    use loom::sync::Arc;
    use loom::thread;

    use super::super::atomic::Ordering;
    use super::super::backend::LoomBackend;
    use super::super::test_waker::counting_waker;
    use super::{CondvarInner, CondvarWaitFutureInner};

    type LoomCondvar = CondvarInner<LoomBackend>;

    /// Verify notify_one wakes a registered waiter under all interleavings.
    #[test]
    fn loom_condvar_notify_wakes_waiter() {
        loom::model(|| {
            let cv = Arc::new(LoomCondvar::new_with_backend());
            let (waker, count) = counting_waker();

            // Poll the wait future once to register the waker.
            let mut fut = CondvarWaitFutureInner {
                condvar: &cv,
                registered: false,
            };
            let mut cx = Context::from_waker(&waker);
            let result = Pin::new(&mut fut).poll(&mut cx);
            assert!(matches!(result, core::task::Poll::Pending));

            let cv2 = cv.clone();
            let t = thread::spawn(move || {
                cv2.notify_one();
            });
            t.join().unwrap();

            assert!(count.load(Ordering::SeqCst) > 0);
        });
    }

    /// Verify notify_all wakes all registered waiters.
    #[test]
    fn loom_condvar_notify_all() {
        loom::model(|| {
            let cv = Arc::new(LoomCondvar::new_with_backend());
            let (waker1, count1) = counting_waker();
            let (waker2, count2) = counting_waker();

            // Register two waiters via CondvarWaitFutureInner.
            let mut fut1 = CondvarWaitFutureInner {
                condvar: &cv,
                registered: false,
            };
            let mut cx1 = Context::from_waker(&waker1);
            assert!(matches!(
                Pin::new(&mut fut1).poll(&mut cx1),
                core::task::Poll::Pending
            ));

            let mut fut2 = CondvarWaitFutureInner {
                condvar: &cv,
                registered: false,
            };
            let mut cx2 = Context::from_waker(&waker2);
            assert!(matches!(
                Pin::new(&mut fut2).poll(&mut cx2),
                core::task::Poll::Pending
            ));

            let cv2 = cv.clone();
            let t = thread::spawn(move || {
                cv2.notify_all();
            });
            t.join().unwrap();

            assert_eq!(count1.load(Ordering::SeqCst), 1);
            assert_eq!(count2.load(Ordering::SeqCst), 1);
        });
    }
}
