//! Futex wait queue logic for userspace synchronization.
//!
//! A futex is identified by a `(process_koid, virtual_address)` pair.
//! [`FutexTableInner::wait`] registers a waker that is woken by a
//! corresponding [`FutexTableInner::wake`].
//!
//! This module contains the pure logic, separated from the kernel's
//! global lock wrapper to enable host-side testing.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::task::Waker;

/// Pure futex wait-queue table without locking.
///
/// The kernel wraps this in an `IrqSpinLock` for the global instance.
pub struct FutexTableInner {
    table: BTreeMap<(u64, u64), VecDeque<Waker>>,
}

impl FutexTableInner {
    /// Creates an empty futex table.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            table: BTreeMap::new(),
        }
    }

    /// Register a waker to be woken by a matching [`wake`](Self::wake).
    pub fn wait(&mut self, koid: u64, addr: u64, waker: Waker) {
        self.table.entry((koid, addr)).or_default().push_back(waker);
    }

    /// Wake up to `count` waiters on the given futex address.
    ///
    /// Returns `(actual_wake_count, wakers_to_wake)`. The caller must
    /// wake the returned wakers **after** releasing any lock protecting
    /// this table to avoid holding locks during executor enqueue.
    pub fn wake(&mut self, koid: u64, addr: u64, count: u32) -> (usize, Vec<Waker>) {
        let Some(waiters) = self.table.get_mut(&(koid, addr)) else {
            return (0, Vec::new());
        };

        let wake_count = (count as usize).min(waiters.len());
        let to_wake: Vec<Waker> = waiters.drain(..wake_count).collect();

        if waiters.is_empty() {
            self.table.remove(&(koid, addr));
        }

        (wake_count, to_wake)
    }

    /// Returns `true` if the table contains an entry for the given key.
    #[cfg(any(test, kani))]
    pub fn contains_key(&self, koid: u64, addr: u64) -> bool {
        self.table.contains_key(&(koid, addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::task::{RawWaker, RawWakerVTable};

    fn counting_waker() -> (Waker, Arc<AtomicU32>) {
        static VTABLE: RawWakerVTable =
            RawWakerVTable::new(clone_fn, wake_fn, wake_by_ref_fn, drop_fn);

        unsafe fn clone_fn(data: *const ()) -> RawWaker {
            let arc = unsafe { Arc::from_raw(data.cast::<AtomicU32>()) };
            let cloned = arc.clone();
            core::mem::forget(arc);
            RawWaker::new(Arc::into_raw(cloned).cast::<()>(), &VTABLE)
        }

        unsafe fn wake_fn(data: *const ()) {
            let arc = unsafe { Arc::from_raw(data.cast::<AtomicU32>()) };
            arc.fetch_add(1, Ordering::Relaxed);
        }

        unsafe fn wake_by_ref_fn(data: *const ()) {
            let arc = unsafe { Arc::from_raw(data.cast::<AtomicU32>()) };
            arc.fetch_add(1, Ordering::Relaxed);
            core::mem::forget(arc);
        }

        unsafe fn drop_fn(data: *const ()) {
            let _ = unsafe { Arc::from_raw(data.cast::<AtomicU32>()) };
        }

        let counter = Arc::new(AtomicU32::new(0));
        let data = Arc::into_raw(counter.clone()).cast::<()>();
        // SAFETY: data is a valid Arc<AtomicU32> pointer; vtable handles refcount.
        let waker = unsafe { Waker::from_raw(RawWaker::new(data, &VTABLE)) };
        (waker, counter)
    }

    #[test]
    fn futex_wake_returns_correct_count() {
        let mut table = FutexTableInner::new();
        let (w1, _) = counting_waker();
        let (w2, _) = counting_waker();
        let (w3, _) = counting_waker();
        table.wait(1, 0x1000, w1);
        table.wait(1, 0x1000, w2);
        table.wait(1, 0x1000, w3);

        let (count, wakers) = table.wake(1, 0x1000, 2);
        assert_eq!(count, 2);
        assert_eq!(wakers.len(), 2);
    }

    #[test]
    fn futex_wake_removes_empty_entry() {
        let mut table = FutexTableInner::new();
        let (w, _) = counting_waker();
        table.wait(1, 0x1000, w);

        let (count, wakers) = table.wake(1, 0x1000, 1);
        assert_eq!(count, 1);
        assert_eq!(wakers.len(), 1);
        assert!(!table.contains_key(1, 0x1000));
    }

    #[test]
    fn futex_no_lost_wakers() {
        let mut table = FutexTableInner::new();
        let wakers: Vec<_> = (0..3).map(|_| counting_waker()).collect();
        for (w, _) in &wakers {
            table.wait(1, 0x1000, w.clone());
        }

        let (count, to_wake) = table.wake(1, 0x1000, 3);
        assert_eq!(count, 3);

        for waker in to_wake {
            waker.wake();
        }

        for (_, counter) in &wakers {
            assert_eq!(counter.load(Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn futex_wake_nonexistent_returns_zero() {
        let mut table = FutexTableInner::new();
        let (count, wakers) = table.wake(42, 0xDEAD, 5);
        assert_eq!(count, 0);
        assert!(wakers.is_empty());
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;
    use core::task::{RawWaker, RawWakerVTable};

    /// A no-op waker since Kani cannot do real waker dispatch.
    fn noop_waker() -> Waker {
        static VTABLE: RawWakerVTable =
            RawWakerVTable::new(|p| RawWaker::new(p, &VTABLE), |_| {}, |_| {}, |_| {});
        // SAFETY: noop vtable is valid for all operations.
        unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
    }

    #[kani::proof]
    fn kani_futex_wake_count_correct() {
        let n: u32 = kani::any();
        kani::assume(n <= 4);
        let m: u32 = kani::any();
        kani::assume(m <= 4);

        let mut table = FutexTableInner::new();
        for _ in 0..n {
            table.wait(1, 0x1000, noop_waker());
        }
        let (count, wakers) = table.wake(1, 0x1000, m);
        let expected = core::cmp::min(n as usize, m as usize);
        assert_eq!(count, expected);
        assert_eq!(wakers.len(), expected);
    }

    #[kani::proof]
    fn kani_futex_wake_removes_when_empty() {
        let mut table = FutexTableInner::new();
        table.wait(1, 0x1000, noop_waker());
        let (count, _) = table.wake(1, 0x1000, 1);
        assert_eq!(count, 1);
        assert!(!table.contains_key(1, 0x1000));
    }

    #[kani::proof]
    fn kani_futex_no_lost_waker() {
        let n: u32 = kani::any();
        kani::assume(n > 0 && n <= 4);

        let mut table = FutexTableInner::new();
        for _ in 0..n {
            table.wait(1, 0x1000, noop_waker());
        }
        let (count, wakers) = table.wake(1, 0x1000, n);
        assert_eq!(count, n as usize);
        assert_eq!(wakers.len(), n as usize);
    }
}

#[cfg(shuttle)]
mod shuttle_tests {
    use shuttle::sync::Arc;
    use shuttle::thread;

    use super::*;

    fn noop_waker() -> Waker {
        use core::task::{RawWaker, RawWakerVTable};
        static VTABLE: RawWakerVTable =
            RawWakerVTable::new(|p| RawWaker::new(p, &VTABLE), |_| {}, |_| {}, |_| {});
        // SAFETY: noop vtable is valid for all operations.
        unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
    }

    #[test]
    fn shuttle_futex_concurrent_wait_wake() {
        shuttle::check_random(
            || {
                let table = Arc::new(shuttle::sync::Mutex::new(FutexTableInner::new()));

                let table2 = table.clone();
                let t1 = thread::spawn(move || {
                    let mut t = table2.lock().unwrap();
                    t.wait(1, 0x1000, noop_waker());
                    t.wait(1, 0x1000, noop_waker());
                });

                let table3 = table.clone();
                let t2 = thread::spawn(move || {
                    let mut t = table3.lock().unwrap();
                    let (count, wakers) = t.wake(1, 0x1000, 5);
                    for w in wakers {
                        w.wake();
                    }
                    count
                });

                t1.join().unwrap();
                let woken = t2.join().unwrap();
                // Depending on schedule: 0 or 2 woken.
                assert!(woken <= 2);
            },
            200,
        );
    }
}
