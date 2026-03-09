//! Spin-based mutual exclusion lock.
//!
//! Uses test-and-test-and-set (TTAS) to reduce cache-line contention.

use core::ops::{Deref, DerefMut};
use core::sync::atomic::Ordering;

use super::backend::{AtomicBoolOps, Backend, CoreBackend, UnsafeCellOps};

#[cfg(hadron_lockdep)]
use super::lockdep::LockClassId;

// ─── Type aliases ─────────────────────────────────────────────────────

/// A spin-based mutual exclusion lock.
///
/// Uses test-and-test-and-set (TTAS) to reduce cache-line contention.
/// Const-constructable so it can be placed in `static` items.
pub type SpinLock<T> = SpinLockInner<T, CoreBackend>;

/// RAII guard that releases the [`SpinLock`] when dropped.
pub type SpinLockGuard<'a, T> = SpinLockGuardInner<'a, T, CoreBackend>;

// ─── Generic inner type ───────────────────────────────────────────────

/// Backend-generic spin lock.
///
/// Use [`SpinLock`] (the [`CoreBackend`] alias) in production code.
pub struct SpinLockInner<T, B: Backend> {
    locked: B::AtomicBool,
    #[cfg(hadron_lockdep)]
    name: &'static str,
    #[cfg(hadron_lockdep)]
    level: u8,
    data: B::UnsafeCell<T>,
}

// SAFETY: The SpinLock ensures exclusive access to `T` via atomic operations.
// `T: Send` is required because the data may be accessed from different threads.
unsafe impl<T: Send, B: Backend> Send for SpinLockInner<T, B> {}
unsafe impl<T: Send, B: Backend> Sync for SpinLockInner<T, B> {}

// ─── Const constructors (CoreBackend only) ────────────────────────────

impl<T> SpinLock<T> {
    /// Creates a new unlocked `SpinLock` wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            #[cfg(hadron_lockdep)]
            level: 0,
            data: core::cell::UnsafeCell::new(value),
        }
    }

    /// Creates a new unlocked `SpinLock` with a name for lockdep diagnostics.
    pub const fn named(name: &'static str, value: T) -> Self {
        let _ = name;
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            #[cfg(hadron_lockdep)]
            name,
            #[cfg(hadron_lockdep)]
            level: 0,
            data: core::cell::UnsafeCell::new(value),
        }
    }

    /// Creates a new unlocked `SpinLock` with a name and lock ordering level.
    ///
    /// `level` is used for lockdep ordering checks: a lock at level N may
    /// only be acquired while holding locks at levels <= N.
    /// Level 0 means "unassigned" (no ordering check).
    pub const fn leveled(name: &'static str, level: u8, value: T) -> Self {
        let _ = (name, level);
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            #[cfg(hadron_lockdep)]
            name,
            #[cfg(hadron_lockdep)]
            level,
            data: core::cell::UnsafeCell::new(value),
        }
    }
}

// ─── Generic non-const constructor ────────────────────────────────────

impl<T, B: Backend> SpinLockInner<T, B> {
    /// Creates a new unlocked `SpinLockInner` using backend factory functions.
    ///
    /// For loom tests and other non-`CoreBackend` backends.
    pub fn new_with_backend(value: T) -> Self {
        Self {
            locked: B::new_atomic_bool(false),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            #[cfg(hadron_lockdep)]
            level: 0,
            data: B::new_unsafe_cell(value),
        }
    }
}

// ─── Algorithm (generic over B) ───────────────────────────────────────

impl<T, B: Backend> SpinLockInner<T, B> {
    /// Acquires the lock, spinning until it becomes available.
    ///
    /// Returns a [`SpinLockGuardInner`] that releases the lock when dropped.
    pub fn lock(&self) -> SpinLockGuardInner<'_, T, B> {
        #[cfg(hadron_lock_debug)]
        {
            if super::irq_spinlock::irq_lock_depth() != 0 {
                panic!("SpinLock::lock() called while holding IrqSpinLock");
            }
        }

        #[cfg(hadron_lock_stress)]
        super::stress::stress_delay();

        loop {
            // Fast path: try to acquire directly.
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                #[cfg(hadron_lockdep)]
                let class = self.lockdep_acquire();

                return SpinLockGuardInner {
                    lock: self,
                    #[cfg(hadron_lockdep)]
                    class,
                };
            }

            // TTAS: spin on a read (shared cache line) until it looks free.
            while self.locked.load(Ordering::Relaxed) {
                B::spin_wait_hint();
            }
        }
    }

    /// Attempts to acquire the lock without blocking.
    ///
    /// Returns `Some(guard)` if the lock was acquired, `None` if it was already held.
    /// Useful in panic handlers where blocking would risk deadlock.
    pub fn try_lock(&self) -> Option<SpinLockGuardInner<'_, T, B>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(hadron_lockdep)]
            let class = self.lockdep_acquire();

            Some(SpinLockGuardInner {
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
    pub fn lock_unchecked(&self) -> SpinLockGuardInner<'_, T, B> {
        loop {
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return SpinLockGuardInner {
                    lock: self,
                    #[cfg(hadron_lockdep)]
                    class: LockClassId::NONE,
                };
            }

            while self.locked.load(Ordering::Relaxed) {
                B::spin_wait_hint();
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
        // SAFETY: Caller guarantees exclusive access.
        self.data.with_mut(|ptr| unsafe { &mut *ptr })
    }

    /// Registers this lock with lockdep and records the acquisition.
    #[cfg(hadron_lockdep)]
    fn lockdep_acquire(&self) -> LockClassId {
        let class = super::lockdep::get_or_register_leveled(
            self as *const _ as usize,
            self.level,
            self.name,
            super::lockdep::LockKind::SpinLock,
        );
        super::lockdep::lock_acquired(class);
        class
    }
}

// ─── Guard ────────────────────────────────────────────────────────────

/// RAII guard that releases the [`SpinLockInner`] when dropped.
pub struct SpinLockGuardInner<'a, T, B: Backend> {
    lock: &'a SpinLockInner<T, B>,
    #[cfg(hadron_lockdep)]
    class: LockClassId,
}

// !Send — holding a SpinLock guard across .await would block other
// tasks from acquiring the lock while the holding task is suspended.
impl<T, B: Backend> !Send for SpinLockGuardInner<'_, T, B> {}

impl<'a, T, B: Backend> SpinLockGuardInner<'a, T, B> {
    /// Returns a reference to the underlying [`SpinLockInner`].
    ///
    /// Used by [`Condvar::wait`](super::Condvar::wait) to re-acquire after release.
    pub fn lock_ref(&self) -> &'a SpinLockInner<T, B> {
        self.lock
    }
}

impl<T, B: Backend> Deref for SpinLockGuardInner<'_, T, B> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: The guard guarantees exclusive access while it exists.
        self.lock.data.with(|ptr| unsafe { &*ptr })
    }
}

impl<T, B: Backend> DerefMut for SpinLockGuardInner<'_, T, B> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: The guard guarantees exclusive access while it exists.
        self.lock.data.with_mut(|ptr| unsafe { &mut *ptr })
    }
}

impl<T, B: Backend> Drop for SpinLockGuardInner<'_, T, B> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);

        #[cfg(hadron_lock_stress)]
        super::stress::stress_delay();

        #[cfg(hadron_lockdep)]
        if self.class != LockClassId::NONE {
            super::lockdep::lock_released(self.class);
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(all(test, not(loom)))]
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

#[cfg(loom)]
mod loom_tests {
    use loom::sync::Arc;
    use loom::thread;

    use super::SpinLockInner;
    use crate::sync::backend::LoomBackend;

    type LoomSpinLock<T> = SpinLockInner<T, LoomBackend>;

    #[test]
    fn loom_mutual_exclusion() {
        loom::model(|| {
            let lock = Arc::new(LoomSpinLock::new_with_backend(0usize));

            let threads: Vec<_> = (0..2)
                .map(|_| {
                    let lock = lock.clone();
                    thread::spawn(move || {
                        for _ in 0..2 {
                            let mut guard = lock.lock();
                            *guard += 1;
                        }
                    })
                })
                .collect();

            for t in threads {
                t.join().unwrap();
            }

            assert_eq!(*lock.lock(), 4);
        });
    }

    #[test]
    fn loom_try_lock_contention() {
        loom::model(|| {
            let lock = Arc::new(LoomSpinLock::new_with_backend(0usize));

            let l1 = lock.clone();
            let t1 = thread::spawn(move || {
                let mut guard = l1.lock();
                *guard += 1;
                // guard dropped here
            });

            let l2 = lock.clone();
            let t2 = thread::spawn(move || {
                // try_lock may or may not succeed depending on interleaving
                if let Some(mut guard) = l2.try_lock() {
                    *guard += 10;
                }
            });

            t1.join().unwrap();
            t2.join().unwrap();

            let val = *lock.lock();
            // t1 always adds 1; t2 adds 10 only if it got the lock
            assert!(val == 1 || val == 11);
        });
    }
}

#[cfg(shuttle)]
mod shuttle_tests {
    use shuttle::sync::Arc;
    use shuttle::thread;

    use super::SpinLockInner;
    use crate::sync::backend::ShuttleBackend;

    type ShuttleSpinLock<T> = SpinLockInner<T, ShuttleBackend>;

    #[test]
    fn shuttle_three_thread_mutual_exclusion() {
        shuttle::check_random(
            || {
                let lock = Arc::new(ShuttleSpinLock::new_with_backend(0usize));

                let threads: Vec<_> = (0..3)
                    .map(|_| {
                        let lock = lock.clone();
                        thread::spawn(move || {
                            for _ in 0..5 {
                                let mut guard = lock.lock();
                                *guard += 1;
                            }
                        })
                    })
                    .collect();

                for t in threads {
                    t.join().unwrap();
                }

                assert_eq!(*lock.lock(), 15);
            },
            100,
        );
    }

    #[test]
    fn shuttle_try_lock_three_threads() {
        shuttle::check_random(
            || {
                let lock = Arc::new(ShuttleSpinLock::new_with_backend(0usize));
                let success_count = Arc::new(shuttle::sync::atomic::AtomicUsize::new(0));

                let threads: Vec<_> = (0..3)
                    .map(|_| {
                        let lock = lock.clone();
                        let count = success_count.clone();
                        thread::spawn(move || {
                            if let Some(mut guard) = lock.try_lock() {
                                *guard += 1;
                                count.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
                            }
                        })
                    })
                    .collect();

                for t in threads {
                    t.join().unwrap();
                }

                let val = *lock.lock();
                let successes = success_count.load(core::sync::atomic::Ordering::SeqCst);
                assert_eq!(val, successes);
            },
            100,
        );
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Verify that `try_lock` returns `None` when the lock is already held.
    #[kani::proof]
    fn spinlock_try_lock_semantics() {
        let lock = SpinLock::new(0u32);
        let guard = lock.try_lock();
        assert!(guard.is_some());
        // While held, a second try_lock must fail.
        assert!(lock.try_lock().is_none());
        drop(guard);
        // After release, try_lock must succeed again.
        assert!(lock.try_lock().is_some());
    }

    /// Verify that data written under the lock is visible after re-acquisition.
    #[kani::proof]
    fn spinlock_protects_data() {
        let val: u32 = kani::any();
        let lock = SpinLock::new(0u32);
        {
            let mut guard = lock.lock();
            *guard = val;
        }
        let guard = lock.lock();
        assert_eq!(*guard, val);
    }
}
