//! Sequence lock for optimistic lock-free reads.
//!
//! [`SeqLock`] allows multiple lock-free readers and a single exclusive
//! writer. Readers retry if they observe a write in progress. Best for
//! small, frequently-read, infrequently-written data (e.g., clock values).

use core::sync::atomic::Ordering;

use super::backend::{AtomicIntOps, Backend, CoreBackend, UnsafeCellOps};

// ─── Type aliases ─────────────────────────────────────────────────────

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
pub type SeqLock<T> = SeqLockInner<T, CoreBackend>;

/// RAII guard for exclusive write access.
pub type SeqLockWriteGuard<'a, T> = SeqLockWriteGuardInner<'a, T, CoreBackend>;

// ─── Generic inner type ───────────────────────────────────────────────

/// Backend-generic sequence lock.
pub struct SeqLockInner<T: Copy, B: Backend> {
    /// Sequence number. Even = unlocked, odd = write in progress.
    seq: B::AtomicU32,
    data: B::UnsafeCell<T>,
}

// SAFETY: SeqLock ensures readers only see consistent data (via retry)
// and writers have exclusive access.
unsafe impl<T: Copy + Send, B: Backend> Send for SeqLockInner<T, B> {}
unsafe impl<T: Copy + Send + Sync, B: Backend> Sync for SeqLockInner<T, B> {}

// ─── Const constructors (CoreBackend only) ────────────────────────────

impl<T: Copy> SeqLock<T> {
    /// Creates a new `SeqLock` wrapping `value`.
    pub const fn new(value: T) -> Self {
        Self {
            seq: core::sync::atomic::AtomicU32::new(0),
            data: core::cell::UnsafeCell::new(value),
        }
    }
}

// ─── Generic non-const constructor ────────────────────────────────────

impl<T: Copy, B: Backend> SeqLockInner<T, B> {
    /// Creates a new `SeqLockInner` using backend factory functions.
    pub fn new_with_backend(value: T) -> Self {
        Self {
            seq: B::new_atomic_u32(0),
            data: B::new_unsafe_cell(value),
        }
    }
}

// ─── Algorithm (generic over B) ───────────────────────────────────────

impl<T: Copy, B: Backend> SeqLockInner<T, B> {
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
                B::spin_wait_hint();
                continue;
            }

            // Copy the data.
            // SAFETY: We've verified no write is in progress. Even if a write
            // starts during this copy, the sequence check below will catch it.
            let value = self.data.with(|ptr| unsafe { *ptr });

            // Re-read sequence number. If it matches s1, the copy is valid.
            let s2 = self.seq.load(Ordering::Acquire);
            if s1 == s2 {
                return value;
            }

            // Sequence changed — a write occurred during our read. Retry.
            B::spin_wait_hint();
        }
    }

    /// Acquires exclusive write access.
    pub fn write(&self) -> SeqLockWriteGuardInner<'_, T, B> {
        // Spin until we can transition from even to odd.
        loop {
            let s = self.seq.load(Ordering::Relaxed);
            if s & 1 != 0 {
                // Another writer is active — spin.
                B::spin_wait_hint();
                continue;
            }
            if self
                .seq
                .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return SeqLockWriteGuardInner { lock: self };
            }
        }
    }
}

// ─── Write guard ──────────────────────────────────────────────────────

/// RAII guard for exclusive write access to a [`SeqLockInner`].
pub struct SeqLockWriteGuardInner<'a, T: Copy, B: Backend> {
    lock: &'a SeqLockInner<T, B>,
}

impl<T: Copy, B: Backend> core::ops::Deref for SeqLockWriteGuardInner<'_, T, B> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: We hold exclusive write access.
        self.lock.data.with(|ptr| unsafe { &*ptr })
    }
}

impl<T: Copy, B: Backend> core::ops::DerefMut for SeqLockWriteGuardInner<'_, T, B> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: We hold exclusive write access.
        self.lock.data.with_mut(|ptr| unsafe { &mut *ptr })
    }
}

impl<T: Copy, B: Backend> Drop for SeqLockWriteGuardInner<'_, T, B> {
    fn drop(&mut self) {
        // Increment sequence to next even number, signaling write complete.
        self.lock.seq.fetch_add(1, Ordering::Release);
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(all(test, not(loom)))]
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

#[cfg(shuttle)]
mod shuttle_tests {
    use shuttle::sync::Arc;
    use shuttle::thread;

    use super::SeqLockInner;
    use crate::sync::backend::ShuttleBackend;

    type ShuttleSeqLock<T> = SeqLockInner<T, ShuttleBackend>;

    #[test]
    fn shuttle_writer_and_readers() {
        shuttle::check_random(
            || {
                let lock = Arc::new(ShuttleSeqLock::new_with_backend(0u64));

                // 1 writer, 3 readers.
                let w = {
                    let lock = lock.clone();
                    thread::spawn(move || {
                        let mut guard = lock.write();
                        *guard = 42;
                    })
                };

                let readers: Vec<_> = (0..3)
                    .map(|_| {
                        let lock = lock.clone();
                        thread::spawn(move || {
                            let val = lock.read();
                            // Readers see either 0 (before write) or 42 (after write).
                            assert!(val == 0 || val == 42);
                        })
                    })
                    .collect();

                w.join().unwrap();
                for t in readers {
                    t.join().unwrap();
                }

                assert_eq!(lock.read(), 42);
            },
            100,
        );
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Verify that data written under a write guard is visible to subsequent reads.
    #[kani::proof]
    fn seqlock_write_read_consistency() {
        let val: u32 = kani::any();
        let lock = SeqLock::new(0u32);
        {
            let mut guard = lock.write();
            *guard = val;
        }
        assert_eq!(lock.read(), val);
    }

    /// Verify sequence number is even after write completes (stable state).
    #[kani::proof]
    fn seqlock_even_after_write() {
        let lock = SeqLock::new(0u32);
        {
            let mut guard = lock.write();
            *guard = 42;
        }
        // After write completes, read should succeed (sequence is even/stable).
        assert_eq!(lock.read(), 42);
    }
}
