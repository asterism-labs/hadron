//! Async-aware mutual exclusion lock.
//!
//! Unlike [`SpinLock`](crate::sync::SpinLock), [`Mutex`] yields the current
//! task via [`WaitQueue`] when contended, allowing the executor to schedule
//! other work. Const-constructable for use in `static` items.

use core::cell::UnsafeCell;
use core::future::Future;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll};

use crate::sync::WaitQueue;

#[cfg(hadron_lockdep)]
use super::lockdep::LockClassId;

/// An async-aware mutual exclusion lock.
///
/// When contended, waiting tasks yield to the executor and are woken via
/// [`WaitQueue`] when the lock becomes available. This avoids wasting CPU
/// cycles spinning in async contexts.
///
/// # Example
///
/// ```ignore
/// static COUNTER: Mutex<u64> = Mutex::new(0);
///
/// async fn increment() {
///     let mut guard = COUNTER.lock().await;
///     *guard += 1;
/// }
/// ```
pub struct Mutex<T> {
    locked: AtomicBool,
    waiters: WaitQueue,
    #[cfg(hadron_lockdep)]
    name: &'static str,
    data: UnsafeCell<T>,
}

// SAFETY: The Mutex ensures exclusive access to `T` via atomic operations.
// `T: Send` is required because the data may be accessed from different threads.
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// Creates a new unlocked `Mutex` wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            waiters: WaitQueue::new(),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            data: UnsafeCell::new(value),
        }
    }

    /// Creates a new unlocked `Mutex` with a name for lockdep diagnostics.
    pub const fn named(name: &'static str, value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            waiters: WaitQueue::new(),
            #[cfg(hadron_lockdep)]
            name,
            data: UnsafeCell::new(value),
        }
    }

    /// Asynchronously acquires the lock.
    ///
    /// Returns a future that resolves to a [`MutexGuard`] once the lock is
    /// acquired. If the lock is already held, the current task yields and is
    /// woken when the lock becomes available.
    pub fn lock(&self) -> MutexLockFuture<'_, T> {
        MutexLockFuture { mutex: self }
    }

    /// Attempts to acquire the lock without blocking.
    ///
    /// Returns `Some(guard)` if the lock was acquired, `None` if it was
    /// already held.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        #[cfg(hadron_lock_debug)]
        {
            let depth = super::irq_spinlock::irq_lock_depth();
            if depth != 0 {
                panic!(
                    "Mutex::try_lock() called while holding {} IrqSpinLock(s)",
                    depth
                );
            }
        }

        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(hadron_lockdep)]
            let class = self.lockdep_acquire();

            Some(MutexGuard {
                mutex: self,
                #[cfg(hadron_lockdep)]
                class,
            })
        } else {
            None
        }
    }

    /// Acquires the lock synchronously by spinning.
    ///
    /// Intended for use during initialization or in contexts where async is
    /// not available. Prefer [`lock`](Mutex::lock) in async contexts.
    pub fn lock_sync(&self) -> MutexGuard<'_, T> {
        #[cfg(hadron_lock_debug)]
        {
            let depth = super::irq_spinlock::irq_lock_depth();
            if depth != 0 {
                panic!(
                    "Mutex::lock_sync() called while holding {} IrqSpinLock(s)",
                    depth
                );
            }
        }

        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            core::hint::spin_loop();
        }
    }

    /// Registers this lock with lockdep and records the acquisition.
    #[cfg(hadron_lockdep)]
    fn lockdep_acquire(&self) -> LockClassId {
        let class = super::lockdep::get_or_register(
            self as *const _ as usize,
            self.name,
            super::lockdep::LockKind::Mutex,
        );
        super::lockdep::lock_acquired(class);
        class
    }
}

/// Future returned by [`Mutex::lock`].
pub struct MutexLockFuture<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<'a, T> Future for MutexLockFuture<'a, T> {
    type Output = MutexGuard<'a, T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        #[cfg(hadron_lock_debug)]
        {
            let depth = super::irq_spinlock::irq_lock_depth();
            if depth != 0 {
                panic!(
                    "Mutex::lock() polled while holding {} IrqSpinLock(s)",
                    depth
                );
            }
        }

        // Fast path: try to acquire directly.
        if self
            .mutex
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(hadron_lockdep)]
            let class = self.mutex.lockdep_acquire();

            return Poll::Ready(MutexGuard {
                mutex: self.mutex,
                #[cfg(hadron_lockdep)]
                class,
            });
        }

        // Register waker BEFORE retry to avoid lost wakeup.
        let registered = self.mutex.waiters.register_waker(cx.waker());

        // Retry after registration — the lock may have been released between
        // our first attempt and the waker registration.
        if self
            .mutex
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(hadron_lockdep)]
            let class = self.mutex.lockdep_acquire();

            return Poll::Ready(MutexGuard {
                mutex: self.mutex,
                #[cfg(hadron_lockdep)]
                class,
            });
        }

        // If WaitQueue was full, self-wake to degrade to spin-poll.
        if !registered {
            cx.waker().wake_by_ref();
        }

        Poll::Pending
    }
}

/// RAII guard that releases the [`Mutex`] when dropped.
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
    #[cfg(hadron_lockdep)]
    class: LockClassId,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: The guard guarantees exclusive access while it exists.
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: The guard guarantees exclusive access while it exists.
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);

        #[cfg(hadron_lockdep)]
        if self.class != LockClassId::NONE {
            super::lockdep::lock_released(self.class);
        }

        self.mutex.waiters.wake_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_waker::{counting_waker, noop_waker};
    use std::sync::atomic::Ordering;
    use std::task::Context;

    #[test]
    fn try_lock_succeeds_when_free() {
        let mutex = Mutex::new(42);
        let guard = mutex.try_lock();
        assert!(guard.is_some());
        assert_eq!(*guard.unwrap(), 42);
    }

    #[test]
    fn try_lock_fails_when_held() {
        let mutex = Mutex::new(42);
        let _guard = mutex.try_lock().unwrap();
        assert!(mutex.try_lock().is_none());
    }

    #[test]
    fn lock_sync_acquires() {
        let mutex = Mutex::new(0);
        let guard = mutex.lock_sync();
        assert_eq!(*guard, 0);
    }

    #[test]
    fn guard_mutate_and_release() {
        let mutex = Mutex::new(0);
        {
            let mut guard = mutex.lock_sync();
            *guard = 99;
        }
        let guard = mutex.lock_sync();
        assert_eq!(*guard, 99);
    }

    #[test]
    fn lock_future_ready_when_free() {
        let mutex = Mutex::new(42);
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut fut = mutex.lock();
        let result = Pin::new(&mut fut).poll(&mut cx);
        assert!(matches!(result, Poll::Ready(_)));
    }

    #[test]
    fn lock_future_pending_when_held() {
        let mutex = Mutex::new(42);
        let _guard = mutex.try_lock().unwrap();

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut fut = mutex.lock();
        let result = Pin::new(&mut fut).poll(&mut cx);
        assert!(matches!(result, Poll::Pending));
    }

    #[test]
    fn drop_guard_wakes_waiter() {
        let mutex = Mutex::new(42);
        let guard = mutex.try_lock().unwrap();

        // Register a counting waker via the lock future.
        let (waker, count) = counting_waker();
        let mut cx = Context::from_waker(&waker);
        let mut fut = mutex.lock();
        let result = Pin::new(&mut fut).poll(&mut cx);
        assert!(matches!(result, Poll::Pending));

        // Drop the guard — should call wake_one and increment our counter.
        drop(guard);
        assert!(
            count.load(Ordering::SeqCst) > 0,
            "waker should have been called"
        );
    }
}
