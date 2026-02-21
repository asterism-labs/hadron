//! Spin-based mutual exclusion lock.
//!
//! Uses test-and-test-and-set (TTAS) to reduce cache-line contention.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(hadron_lockdep)]
use super::lockdep::LockClassId;

/// A spin-based mutual exclusion lock.
///
/// Uses test-and-test-and-set (TTAS) to reduce cache-line contention.
/// Const-constructable so it can be placed in `static` items.
pub struct SpinLock<T> {
    locked: AtomicBool,
    #[cfg(hadron_lockdep)]
    name: &'static str,
    data: UnsafeCell<T>,
}

// SAFETY: The SpinLock ensures exclusive access to `T` via atomic operations.
// `T: Send` is required because the data may be accessed from different threads.
unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Creates a new unlocked `SpinLock` wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            data: UnsafeCell::new(value),
        }
    }

    /// Creates a new unlocked `SpinLock` with a name for lockdep diagnostics.
    pub const fn named(name: &'static str, value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            #[cfg(hadron_lockdep)]
            name,
            data: UnsafeCell::new(value),
        }
    }

    /// Acquires the lock, spinning until it becomes available.
    ///
    /// Returns a [`SpinLockGuard`] that releases the lock when dropped.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        #[cfg(hadron_lock_debug)]
        {
            if super::irq_spinlock::irq_lock_depth() != 0 {
                panic!("SpinLock::lock() called while holding IrqSpinLock");
            }
        }

        loop {
            // Fast path: try to acquire directly.
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                #[cfg(hadron_lockdep)]
                let class = self.lockdep_acquire();

                return SpinLockGuard {
                    lock: self,
                    #[cfg(hadron_lockdep)]
                    class,
                };
            }

            // TTAS: spin on a read (shared cache line) until it looks free.
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
    }

    /// Attempts to acquire the lock without blocking.
    ///
    /// Returns `Some(guard)` if the lock was acquired, `None` if it was already held.
    /// Useful in panic handlers where blocking would risk deadlock.
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(hadron_lockdep)]
            let class = self.lockdep_acquire();

            Some(SpinLockGuard {
                lock: self,
                #[cfg(hadron_lockdep)]
                class,
            })
        } else {
            None
        }
    }

    /// Acquires the lock without the IRQ-context assertion.
    ///
    /// Only for locks known-safe to hold with interrupts disabled — specifically
    /// the heap allocator, which may be entered from any context including
    /// `IrqSpinLock` critical sections that allocate.
    pub fn lock_unchecked(&self) -> SpinLockGuard<'_, T> {
        loop {
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return SpinLockGuard {
                    lock: self,
                    #[cfg(hadron_lockdep)]
                    class: LockClassId::NONE,
                };
            }

            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
    }

    /// Returns a mutable reference to the underlying data without acquiring the lock.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other code is concurrently accessing the data.
    /// Intended as a last-resort escape hatch (e.g., panic handler on a uniprocessor).
    pub unsafe fn force_get(&self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }

    /// Registers this lock with lockdep and records the acquisition.
    #[cfg(hadron_lockdep)]
    fn lockdep_acquire(&self) -> LockClassId {
        let class = super::lockdep::get_or_register(
            self as *const _ as usize,
            self.name,
            super::lockdep::LockKind::SpinLock,
        );
        super::lockdep::lock_acquired(class);
        class
    }
}

/// RAII guard that releases the [`SpinLock`] when dropped.
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    #[cfg(hadron_lockdep)]
    class: LockClassId,
}

impl<'a, T> SpinLockGuard<'a, T> {
    /// Returns a reference to the underlying [`SpinLock`].
    ///
    /// Used by [`Condvar::wait`](super::Condvar::wait) to re-acquire after release.
    pub fn lock_ref(&self) -> &'a SpinLock<T> {
        self.lock
    }
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: The guard guarantees exclusive access while it exists.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: The guard guarantees exclusive access while it exists.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);

        #[cfg(hadron_lockdep)]
        if self.class != LockClassId::NONE {
            super::lockdep::lock_released(self.class);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_unlock() {
        let lock = SpinLock::new(42);
        {
            let guard = lock.lock();
            assert_eq!(*guard, 42);
        }
        // Lock is released after guard is dropped.
        let guard = lock.try_lock();
        assert!(guard.is_some());
    }

    #[test]
    fn try_lock_succeeds_when_free() {
        let lock = SpinLock::new(10);
        let guard = lock.try_lock();
        assert!(guard.is_some());
        assert_eq!(*guard.unwrap(), 10);
    }

    #[test]
    fn try_lock_fails_when_held() {
        let lock = SpinLock::new(10);
        let _guard = lock.lock();
        assert!(lock.try_lock().is_none());
    }

    #[test]
    fn mutate_through_guard() {
        let lock = SpinLock::new(0);
        {
            let mut guard = lock.lock();
            *guard = 99;
        }
        let guard = lock.lock();
        assert_eq!(*guard, 99);
    }

    #[test]
    fn lock_reentrant_after_drop() {
        let lock = SpinLock::new(42);
        {
            let _guard = lock.lock();
        }
        // After guard is dropped, we should be able to lock again.
        {
            let _guard = lock.lock();
        }
        // And try_lock should also work.
        assert!(lock.try_lock().is_some());
    }

    #[test]
    fn deref_and_deref_mut() {
        let lock = SpinLock::new(String::from("hello"));
        {
            let guard = lock.lock();
            // Deref: read access
            assert_eq!(guard.len(), 5);
        }
        {
            let mut guard = lock.lock();
            // DerefMut: write access
            guard.push_str(" world");
        }
        let guard = lock.lock();
        assert_eq!(&*guard, "hello world");
    }

    #[test]
    fn named_constructor() {
        let lock = SpinLock::named("test_lock", 42);
        let guard = lock.lock();
        assert_eq!(*guard, 42);
    }
}
