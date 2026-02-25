//! Async-aware mutual exclusion lock.
//!
//! Unlike [`SpinLock`](crate::sync::SpinLock), [`Mutex`] yields the current
//! task via [`WaitQueue`] when contended, allowing the executor to schedule
//! other work. Const-constructable for use in `static` items.

use core::future::Future;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::sync::atomic::Ordering;
use core::task::{Context, Poll};

use super::backend::{AtomicBoolOps, CoreBackend, IrqBackend, UnsafeCellOps};
use super::waitqueue::{WaitQueue, WaitQueueInner};

#[cfg(hadron_lockdep)]
use super::lockdep::LockClassId;

// ─── Type aliases ─────────────────────────────────────────────────────

/// An async-aware mutual exclusion lock.
///
/// When contended, waiting tasks yield to the executor and are woken via
/// [`WaitQueue`] when the lock becomes available.
pub type Mutex<T> = MutexInner<T, CoreBackend>;

/// RAII guard that releases the [`Mutex`] when dropped.
pub type MutexGuard<'a, T> = MutexGuardInner<'a, T, CoreBackend>;

/// Future returned by [`Mutex::lock`].
pub type MutexLockFuture<'a, T> = MutexLockFutureInner<'a, T, CoreBackend>;

// ─── Generic inner type ───────────────────────────────────────────────

/// Backend-generic async-aware mutex.
pub struct MutexInner<T, B: IrqBackend> {
    locked: B::AtomicBool,
    waiters: WaitQueueInner<B>,
    #[cfg(hadron_lockdep)]
    name: &'static str,
    data: B::UnsafeCell<T>,
}

// SAFETY: The Mutex ensures exclusive access to `T` via atomic operations.
unsafe impl<T: Send, B: IrqBackend> Send for MutexInner<T, B> {}
unsafe impl<T: Send, B: IrqBackend> Sync for MutexInner<T, B> {}

// ─── Const constructors (CoreBackend only) ────────────────────────────

impl<T> Mutex<T> {
    /// Creates a new unlocked `Mutex` wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            waiters: WaitQueue::new(),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            data: core::cell::UnsafeCell::new(value),
        }
    }

    /// Creates a new unlocked `Mutex` with a name for lockdep diagnostics.
    pub const fn named(name: &'static str, value: T) -> Self {
        let _ = name;
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            waiters: WaitQueue::new(),
            #[cfg(hadron_lockdep)]
            name,
            data: core::cell::UnsafeCell::new(value),
        }
    }
}

// ─── Generic non-const constructor ────────────────────────────────────

impl<T, B: IrqBackend> MutexInner<T, B> {
    /// Creates a new unlocked `MutexInner` using backend factory functions.
    pub fn new_with_backend(value: T) -> Self {
        Self {
            locked: B::new_atomic_bool(false),
            waiters: WaitQueueInner::new_with_backend(),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            data: B::new_unsafe_cell(value),
        }
    }
}

// ─── Algorithm (generic over B) ───────────────────────────────────────

impl<T, B: IrqBackend> MutexInner<T, B> {
    /// Asynchronously acquires the lock.
    pub fn lock(&self) -> MutexLockFutureInner<'_, T, B> {
        MutexLockFutureInner { mutex: self }
    }

    /// Attempts to acquire the lock without blocking.
    pub fn try_lock(&self) -> Option<MutexGuardInner<'_, T, B>> {
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

            Some(MutexGuardInner {
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
    /// not available. Prefer [`lock`](MutexInner::lock) in async contexts.
    pub fn lock_sync(&self) -> MutexGuardInner<'_, T, B> {
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
            B::spin_wait_hint();
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

// ─── Lock future ──────────────────────────────────────────────────────

/// Future returned by [`MutexInner::lock`].
pub struct MutexLockFutureInner<'a, T, B: IrqBackend> {
    mutex: &'a MutexInner<T, B>,
}

impl<'a, T, B: IrqBackend> Future for MutexLockFutureInner<'a, T, B> {
    type Output = MutexGuardInner<'a, T, B>;

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

        #[cfg(hadron_lock_stress)]
        super::stress::stress_delay();

        // Fast path: try to acquire directly.
        if self
            .mutex
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(hadron_lockdep)]
            let class = self.mutex.lockdep_acquire();

            return Poll::Ready(MutexGuardInner {
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
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(hadron_lockdep)]
            let class = self.mutex.lockdep_acquire();

            return Poll::Ready(MutexGuardInner {
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

// ─── Guard ────────────────────────────────────────────────────────────

/// RAII guard that releases the [`MutexInner`] when dropped.
pub struct MutexGuardInner<'a, T, B: IrqBackend> {
    mutex: &'a MutexInner<T, B>,
    #[cfg(hadron_lockdep)]
    class: LockClassId,
}

impl<'a, T, B: IrqBackend> MutexGuardInner<'a, T, B> {
    /// Returns a reference to the underlying [`MutexInner`].
    ///
    /// Used by [`Condvar::wait_async`](super::Condvar::wait_async) to re-acquire after release.
    pub fn mutex_ref(&self) -> &'a MutexInner<T, B> {
        self.mutex
    }
}

impl<T, B: IrqBackend> Deref for MutexGuardInner<'_, T, B> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: The guard guarantees exclusive access while it exists.
        self.mutex.data.with(|ptr| unsafe { &*ptr })
    }
}

impl<T, B: IrqBackend> DerefMut for MutexGuardInner<'_, T, B> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: The guard guarantees exclusive access while it exists.
        self.mutex.data.with_mut(|ptr| unsafe { &mut *ptr })
    }
}

impl<T, B: IrqBackend> Drop for MutexGuardInner<'_, T, B> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);

        #[cfg(hadron_lockdep)]
        if self.class != LockClassId::NONE {
            super::lockdep::lock_released(self.class);
        }

        self.mutex.waiters.wake_one();
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(all(test, not(loom)))]
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

        let (waker, count) = counting_waker();
        let mut cx = Context::from_waker(&waker);
        let mut fut = mutex.lock();
        let result = Pin::new(&mut fut).poll(&mut cx);
        assert!(matches!(result, Poll::Pending));

        drop(guard);
        assert!(
            count.load(Ordering::SeqCst) > 0,
            "waker should have been called"
        );
    }
}

#[cfg(loom)]
mod loom_tests {
    use loom::sync::Arc;
    use loom::thread;

    use super::MutexInner;
    use crate::sync::backend::LoomBackend;

    type LoomMutex<T> = MutexInner<T, LoomBackend>;

    #[test]
    fn loom_mutex_mutual_exclusion() {
        loom::model(|| {
            let mutex = Arc::new(LoomMutex::new_with_backend(0usize));

            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let m = mutex.clone();
                    thread::spawn(move || {
                        let mut guard = m.lock_sync();
                        *guard += 1;
                    })
                })
                .collect();

            for h in handles {
                h.join().unwrap();
            }

            assert_eq!(*mutex.lock_sync(), 2);
        });
    }

    #[test]
    fn loom_mutex_async_contention() {
        use core::future::Future;
        use core::pin::Pin;
        use core::task::Context;

        loom::model(|| {
            let mutex = Arc::new(LoomMutex::new_with_backend(0usize));

            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let m = mutex.clone();
                    thread::spawn(move || {
                        let waker = crate::sync::test_waker::noop_waker();
                        let mut cx = Context::from_waker(&waker);
                        let mut fut = m.lock();
                        loop {
                            match Pin::new(&mut fut).poll(&mut cx) {
                                core::task::Poll::Ready(mut guard) => {
                                    *guard += 1;
                                    break;
                                }
                                core::task::Poll::Pending => loom::thread::yield_now(),
                            }
                        }
                    })
                })
                .collect();

            for h in handles {
                h.join().unwrap();
            }

            assert_eq!(*mutex.lock_sync(), 2);
        });
    }
}
