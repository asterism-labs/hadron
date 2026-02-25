//! Spinning reader-writer lock.
//!
//! Allows multiple concurrent readers or a single exclusive writer.
//! Uses an `AtomicU32` for state: 0 = unlocked, positive = reader count,
//! `u32::MAX` = write-locked.

use core::ops::{Deref, DerefMut};
use core::sync::atomic::Ordering;

use super::backend::{AtomicIntOps, Backend, CoreBackend, UnsafeCellOps};

#[cfg(hadron_lockdep)]
use super::lockdep::LockClassId;

/// Sentinel value indicating the lock is held exclusively for writing.
const WRITE_LOCKED: u32 = u32::MAX;

// ─── Type aliases ─────────────────────────────────────────────────────

/// A spinning reader-writer lock.
///
/// Const-constructable and suitable for `static` items.
pub type RwLock<T> = RwLockInner<T, CoreBackend>;

/// RAII guard for a shared read lock.
pub type RwLockReadGuard<'a, T> = RwLockReadGuardInner<'a, T, CoreBackend>;

/// RAII guard for an exclusive write lock.
pub type RwLockWriteGuard<'a, T> = RwLockWriteGuardInner<'a, T, CoreBackend>;

// ─── Generic inner type ───────────────────────────────────────────────

/// Backend-generic reader-writer lock.
pub struct RwLockInner<T, B: Backend> {
    state: B::AtomicU32,
    #[cfg(hadron_lockdep)]
    name: &'static str,
    #[cfg(hadron_lockdep)]
    level: u8,
    data: B::UnsafeCell<T>,
}

// SAFETY: The RwLock ensures that `T` is either accessed by multiple shared
// readers (requiring `T: Sync`) or by a single exclusive writer (requiring
// `T: Send`).
unsafe impl<T: Send, B: Backend> Send for RwLockInner<T, B> {}
unsafe impl<T: Send + Sync, B: Backend> Sync for RwLockInner<T, B> {}

// ─── Const constructors (CoreBackend only) ────────────────────────────

impl<T> RwLock<T> {
    /// Creates a new unlocked `RwLock` wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            state: core::sync::atomic::AtomicU32::new(0),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            #[cfg(hadron_lockdep)]
            level: 0,
            data: core::cell::UnsafeCell::new(value),
        }
    }

    /// Creates a new unlocked `RwLock` with a name for lockdep diagnostics.
    pub const fn named(name: &'static str, value: T) -> Self {
        let _ = name;
        Self {
            state: core::sync::atomic::AtomicU32::new(0),
            #[cfg(hadron_lockdep)]
            name,
            #[cfg(hadron_lockdep)]
            level: 0,
            data: core::cell::UnsafeCell::new(value),
        }
    }

    /// Creates a new unlocked `RwLock` with a name and lock ordering level.
    pub const fn leveled(name: &'static str, level: u8, value: T) -> Self {
        let _ = (name, level);
        Self {
            state: core::sync::atomic::AtomicU32::new(0),
            #[cfg(hadron_lockdep)]
            name,
            #[cfg(hadron_lockdep)]
            level,
            data: core::cell::UnsafeCell::new(value),
        }
    }
}

// ─── Generic non-const constructor ────────────────────────────────────

impl<T, B: Backend> RwLockInner<T, B> {
    /// Creates a new unlocked `RwLockInner` using backend factory functions.
    pub fn new_with_backend(value: T) -> Self {
        Self {
            state: B::new_atomic_u32(0),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            #[cfg(hadron_lockdep)]
            level: 0,
            data: B::new_unsafe_cell(value),
        }
    }
}

// ─── Algorithm (generic over B) ───────────────────────────────────────

impl<T, B: Backend> RwLockInner<T, B> {
    /// Acquires a shared read lock, spinning until no writer holds the lock.
    pub fn read(&self) -> RwLockReadGuardInner<'_, T, B> {
        #[cfg(hadron_lock_stress)]
        super::stress::stress_delay();

        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s != WRITE_LOCKED {
                if self
                    .state
                    .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    #[cfg(hadron_lockdep)]
                    let class = self.lockdep_acquire();

                    return RwLockReadGuardInner {
                        lock: self,
                        #[cfg(hadron_lockdep)]
                        class,
                    };
                }
            }
            B::spin_wait_hint();
        }
    }

    /// Acquires an exclusive write lock, spinning until no readers or writers
    /// hold the lock.
    pub fn write(&self) -> RwLockWriteGuardInner<'_, T, B> {
        #[cfg(hadron_lock_stress)]
        super::stress::stress_delay();

        loop {
            if self
                .state
                .compare_exchange_weak(0, WRITE_LOCKED, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                #[cfg(hadron_lockdep)]
                let class = self.lockdep_acquire();

                return RwLockWriteGuardInner {
                    lock: self,
                    #[cfg(hadron_lockdep)]
                    class,
                };
            }
            B::spin_wait_hint();
        }
    }

    /// Tries to acquire a shared read lock without blocking.
    pub fn try_read(&self) -> Option<RwLockReadGuardInner<'_, T, B>> {
        let s = self.state.load(Ordering::Relaxed);
        if s != WRITE_LOCKED {
            if self
                .state
                .compare_exchange(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                #[cfg(hadron_lockdep)]
                let class = self.lockdep_acquire();

                return Some(RwLockReadGuardInner {
                    lock: self,
                    #[cfg(hadron_lockdep)]
                    class,
                });
            }
        }
        None
    }

    /// Tries to acquire an exclusive write lock without blocking.
    pub fn try_write(&self) -> Option<RwLockWriteGuardInner<'_, T, B>> {
        if self
            .state
            .compare_exchange(0, WRITE_LOCKED, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(hadron_lockdep)]
            let class = self.lockdep_acquire();

            Some(RwLockWriteGuardInner {
                lock: self,
                #[cfg(hadron_lockdep)]
                class,
            })
        } else {
            None
        }
    }

    /// Registers this lock with lockdep and records the acquisition.
    #[cfg(hadron_lockdep)]
    fn lockdep_acquire(&self) -> LockClassId {
        let class = super::lockdep::get_or_register_leveled(
            self as *const _ as usize,
            self.level,
            self.name,
            super::lockdep::LockKind::RwLock,
        );
        super::lockdep::lock_acquired(class);
        class
    }
}

// ─── Read guard ───────────────────────────────────────────────────────

/// RAII guard for a shared read lock on an [`RwLockInner`].
///
/// `!Send` — holding across `.await` would block writers while suspended.
pub struct RwLockReadGuardInner<'a, T, B: Backend> {
    lock: &'a RwLockInner<T, B>,
    #[cfg(hadron_lockdep)]
    class: LockClassId,
}

impl<T, B: Backend> !Send for RwLockReadGuardInner<'_, T, B> {}

impl<T, B: Backend> Deref for RwLockReadGuardInner<'_, T, B> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: Read lock is held — no writer can exist.
        self.lock.data.with(|ptr| unsafe { &*ptr })
    }
}

impl<T, B: Backend> Drop for RwLockReadGuardInner<'_, T, B> {
    fn drop(&mut self) {
        self.lock.state.fetch_sub(1, Ordering::Release);

        #[cfg(hadron_lock_stress)]
        super::stress::stress_delay();

        #[cfg(hadron_lockdep)]
        if self.class != LockClassId::NONE {
            super::lockdep::lock_released(self.class);
        }
    }
}

// ─── Write guard ──────────────────────────────────────────────────────

/// RAII guard for an exclusive write lock on an [`RwLockInner`].
///
/// `!Send` — holding across `.await` would block all readers/writers while suspended.
pub struct RwLockWriteGuardInner<'a, T, B: Backend> {
    lock: &'a RwLockInner<T, B>,
    #[cfg(hadron_lockdep)]
    class: LockClassId,
}

impl<T, B: Backend> !Send for RwLockWriteGuardInner<'_, T, B> {}

impl<T, B: Backend> Deref for RwLockWriteGuardInner<'_, T, B> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: Write lock is held — no other reader or writer can exist.
        self.lock.data.with(|ptr| unsafe { &*ptr })
    }
}

impl<T, B: Backend> DerefMut for RwLockWriteGuardInner<'_, T, B> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: Write lock is held — no other reader or writer can exist.
        self.lock.data.with_mut(|ptr| unsafe { &mut *ptr })
    }
}

impl<T, B: Backend> Drop for RwLockWriteGuardInner<'_, T, B> {
    fn drop(&mut self) {
        self.lock.state.store(0, Ordering::Release);

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
    fn read_lock() {
        let lock = RwLock::new(42);
        let guard = lock.read();
        assert_eq!(*guard, 42);
    }

    #[test]
    fn multiple_readers() {
        let lock = RwLock::new(42);
        let g1 = lock.read();
        let g2 = lock.read();
        assert_eq!(*g1, 42);
        assert_eq!(*g2, 42);
    }

    #[test]
    fn write_lock() {
        let lock = RwLock::new(0);
        {
            let mut guard = lock.write();
            *guard = 99;
        }
        let guard = lock.read();
        assert_eq!(*guard, 99);
    }

    #[test]
    fn try_read_succeeds() {
        let lock = RwLock::new(10);
        assert!(lock.try_read().is_some());
    }

    #[test]
    fn try_write_succeeds() {
        let lock = RwLock::new(10);
        assert!(lock.try_write().is_some());
    }

    #[test]
    fn try_write_fails_with_reader() {
        let lock = RwLock::new(10);
        let _reader = lock.read();
        assert!(lock.try_write().is_none());
    }

    #[test]
    fn try_read_fails_with_writer() {
        let lock = RwLock::new(10);
        let _writer = lock.write();
        assert!(lock.try_read().is_none());
    }

    #[test]
    fn multiple_readers_3() {
        let lock = RwLock::new(42);
        let g1 = lock.read();
        let g2 = lock.read();
        let g3 = lock.read();
        assert_eq!(*g1, 42);
        assert_eq!(*g2, 42);
        assert_eq!(*g3, 42);
    }

    #[test]
    fn try_write_with_two_readers() {
        let lock = RwLock::new(42);
        let _g1 = lock.read();
        let _g2 = lock.read();
        // Cannot write while readers exist.
        assert!(lock.try_write().is_none());
    }

    #[test]
    fn write_after_readers_dropped() {
        let lock = RwLock::new(0);
        {
            let _g1 = lock.read();
            let _g2 = lock.read();
        }
        // Readers dropped — write should succeed.
        let mut guard = lock.write();
        *guard = 42;
        drop(guard);
        assert_eq!(*lock.read(), 42);
    }

    #[test]
    fn try_write_with_writer() {
        let lock = RwLock::new(0);
        let _writer = lock.write();
        assert!(lock.try_write().is_none());
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Verify readers and writer cannot coexist.
    #[kani::proof]
    fn rwlock_reader_writer_exclusion() {
        let lock = RwLock::new(0u32);
        let _reader = lock.read();
        // While a reader is held, write must fail.
        assert!(lock.try_write().is_none());
    }

    /// Verify that state is valid: 0 (unlocked), 1..MAX-1 (readers), or MAX (write-locked).
    #[kani::proof]
    fn rwlock_state_invariants() {
        let lock = RwLock::new(0u32);
        // After read, state should be 1.
        let r1 = lock.read();
        assert!(lock.try_write().is_none());
        // After dropping, write should succeed.
        drop(r1);
        let w = lock.try_write();
        assert!(w.is_some());
    }

    /// Verify data written under write lock is visible to subsequent readers.
    #[kani::proof]
    fn rwlock_write_then_read() {
        let val: u32 = kani::any();
        let lock = RwLock::new(0u32);
        {
            let mut guard = lock.write();
            *guard = val;
        }
        let guard = lock.read();
        assert_eq!(*guard, val);
    }
}
