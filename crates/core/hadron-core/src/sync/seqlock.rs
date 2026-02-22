//! Sequence lock for optimistic lock-free reads.
//!
//! [`SeqLock`] allows multiple lock-free readers and a single exclusive
//! writer. Readers retry if they observe a write in progress. Best for
//! small, frequently-read, infrequently-written data (e.g., clock values).

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

/// A sequence lock.
///
/// Writers acquire exclusive access and increment the sequence number.
/// Readers optimistically copy the data and verify the sequence number
/// hasn't changed. If it has, the read is retried.
///
/// # Constraints
///
/// `T` must be `Copy` — readers perform a bitwise copy, which may observe
/// partial writes if the sequence check fails (the copy is discarded in
/// that case).
///
/// # Example
///
/// ```ignore
/// static CLOCK: SeqLock<u64> = SeqLock::new(0);
///
/// // Reader (lock-free):
/// let time = CLOCK.read();
///
/// // Writer (exclusive):
/// let mut guard = CLOCK.write();
/// *guard = 42;
/// ```
pub struct SeqLock<T: Copy> {
    /// Sequence number. Even = unlocked, odd = write in progress.
    seq: AtomicU32,
    data: UnsafeCell<T>,
}

// SAFETY: SeqLock ensures readers only see consistent data (via retry)
// and writers have exclusive access.
unsafe impl<T: Copy + Send> Send for SeqLock<T> {}
unsafe impl<T: Copy + Send + Sync> Sync for SeqLock<T> {}

impl<T: Copy> SeqLock<T> {
    /// Creates a new `SeqLock` wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            seq: AtomicU32::new(0),
            data: UnsafeCell::new(value),
        }
    }

    /// Performs an optimistic lock-free read.
    ///
    /// Retries automatically if a write is in progress. This method never
    /// blocks writers — it simply re-reads until it gets a consistent snapshot.
    pub fn read(&self) -> T {
        loop {
            // Read sequence number (must be even = no write in progress).
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 != 0 {
                // Write in progress — spin and retry.
                core::hint::spin_loop();
                continue;
            }

            // Copy the data.
            // SAFETY: We've verified no write is in progress. Even if a write
            // starts during this copy, the sequence check below will catch it.
            let value = unsafe { *self.data.get() };

            // Re-read sequence number. If it matches s1, the copy is valid.
            let s2 = self.seq.load(Ordering::Acquire);
            if s1 == s2 {
                return value;
            }

            // Sequence changed — a write occurred during our read. Retry.
            core::hint::spin_loop();
        }
    }

    /// Acquires exclusive write access.
    ///
    /// Increments the sequence number to odd (signaling write in progress),
    /// returns a guard that allows mutation, and restores the sequence to
    /// the next even number on drop.
    pub fn write(&self) -> SeqLockWriteGuard<'_, T> {
        // Spin until we can transition from even to odd.
        loop {
            let s = self.seq.load(Ordering::Relaxed);
            if s & 1 != 0 {
                // Another writer is active — spin.
                core::hint::spin_loop();
                continue;
            }
            if self
                .seq
                .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return SeqLockWriteGuard { lock: self };
            }
        }
    }
}

/// RAII guard for exclusive write access to a [`SeqLock`].
pub struct SeqLockWriteGuard<'a, T: Copy> {
    lock: &'a SeqLock<T>,
}

impl<T: Copy> core::ops::Deref for SeqLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: We hold exclusive write access.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: Copy> core::ops::DerefMut for SeqLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: We hold exclusive write access.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: Copy> Drop for SeqLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        // Increment sequence to next even number, signaling write complete.
        self.lock.seq.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_initial_value() {
        let lock = SeqLock::new(42u64);
        assert_eq!(lock.read(), 42);
    }

    #[test]
    fn write_then_read() {
        let lock = SeqLock::new(0u64);
        {
            let mut guard = lock.write();
            *guard = 99;
        }
        assert_eq!(lock.read(), 99);
    }

    #[test]
    fn multiple_writes() {
        let lock = SeqLock::new(0u32);
        for i in 1..=10 {
            let mut guard = lock.write();
            *guard = i;
        }
        assert_eq!(lock.read(), 10);
    }

    #[test]
    fn read_without_contention() {
        let lock = SeqLock::new(123u64);
        // Multiple reads should all succeed and return the same value.
        for _ in 0..100 {
            assert_eq!(lock.read(), 123);
        }
    }

    #[test]
    fn sequence_number_progression() {
        let lock = SeqLock::new(0u32);
        // Initial sequence is 0 (even).
        assert_eq!(lock.seq.load(Ordering::Relaxed), 0);

        {
            let _guard = lock.write();
            // During write, sequence is odd.
            assert_eq!(lock.seq.load(Ordering::Relaxed), 1);
        }
        // After write, sequence is 2 (even).
        assert_eq!(lock.seq.load(Ordering::Relaxed), 2);

        {
            let _guard = lock.write();
            assert_eq!(lock.seq.load(Ordering::Relaxed), 3);
        }
        assert_eq!(lock.seq.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn write_guard_deref() {
        let lock = SeqLock::new(42u64);
        let guard = lock.write();
        assert_eq!(*guard, 42);
    }

    #[test]
    fn write_guard_deref_mut() {
        let lock = SeqLock::new(0u64);
        {
            let mut guard = lock.write();
            *guard = 7;
        }
        assert_eq!(lock.read(), 7);
    }

    #[test]
    fn copy_type_struct() {
        #[derive(Clone, Copy, PartialEq, Debug)]
        struct Pair {
            a: u32,
            b: u32,
        }

        let lock = SeqLock::new(Pair { a: 1, b: 2 });
        assert_eq!(lock.read(), Pair { a: 1, b: 2 });

        {
            let mut guard = lock.write();
            *guard = Pair { a: 10, b: 20 };
        }
        assert_eq!(lock.read(), Pair { a: 10, b: 20 });
    }
}
