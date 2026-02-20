//! Spinning reader-writer lock.
//!
//! Allows multiple concurrent readers or a single exclusive writer.
//! Uses an `AtomicU32` for state: 0 = unlocked, positive = reader count,
//! `u32::MAX` = write-locked.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};

/// Sentinel value indicating the lock is held exclusively for writing.
const WRITE_LOCKED: u32 = u32::MAX;

/// A spinning reader-writer lock.
///
/// Const-constructable and suitable for `static` items.
pub struct RwLock<T> {
    state: AtomicU32,
    data: UnsafeCell<T>,
}

// SAFETY: The RwLock ensures that `T` is either accessed by multiple shared
// readers (requiring `T: Sync`) or by a single exclusive writer (requiring
// `T: Send`).
unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    /// Creates a new unlocked `RwLock` wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            state: AtomicU32::new(0),
            data: UnsafeCell::new(value),
        }
    }

    /// Acquires a shared read lock, spinning until no writer holds the lock.
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s != WRITE_LOCKED {
                if self
                    .state
                    .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    return RwLockReadGuard { lock: self };
                }
            }
            core::hint::spin_loop();
        }
    }

    /// Acquires an exclusive write lock, spinning until no readers or writers
    /// hold the lock.
    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        loop {
            if self
                .state
                .compare_exchange_weak(0, WRITE_LOCKED, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return RwLockWriteGuard { lock: self };
            }
            core::hint::spin_loop();
        }
    }

    /// Tries to acquire a shared read lock without blocking.
    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
        let s = self.state.load(Ordering::Relaxed);
        if s != WRITE_LOCKED {
            if self
                .state
                .compare_exchange(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(RwLockReadGuard { lock: self });
            }
        }
        None
    }

    /// Tries to acquire an exclusive write lock without blocking.
    pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
        if self
            .state
            .compare_exchange(0, WRITE_LOCKED, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(RwLockWriteGuard { lock: self })
        } else {
            None
        }
    }
}

/// RAII guard for a shared read lock on an [`RwLock`].
pub struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: Read lock is held — no writer can exist.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.fetch_sub(1, Ordering::Release);
    }
}

/// RAII guard for an exclusive write lock on an [`RwLock`].
pub struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: Write lock is held — no other reader or writer can exist.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: Write lock is held — no other reader or writer can exist.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.store(0, Ordering::Release);
    }
}

#[cfg(test)]
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
