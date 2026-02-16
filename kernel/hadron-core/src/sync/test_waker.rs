//! Test waker utilities for async unit tests.
//!
//! Provides [`noop_waker`] and [`counting_waker`] for polling futures
//! in host-side tests without a real executor.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{RawWaker, RawWakerVTable, Waker};

/// Creates a [`Waker`] that does nothing when woken.
pub fn noop_waker() -> Waker {
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
}

/// Creates a [`Waker`] that increments a counter each time it is woken.
///
/// Returns the waker and an `Arc<AtomicUsize>` counter that tracks wake calls.
pub fn counting_waker() -> (Waker, Arc<AtomicUsize>) {
    let counter = Arc::new(AtomicUsize::new(0));
    let data = Arc::into_raw(counter.clone()) as *const ();

    unsafe fn clone(data: *const ()) -> RawWaker {
        // SAFETY: `data` is a valid `Arc<AtomicUsize>` pointer created by `Arc::into_raw`.
        let arc = unsafe { Arc::from_raw(data as *const AtomicUsize) };
        let cloned = arc.clone();
        let _ = Arc::into_raw(arc); // don't drop original
        RawWaker::new(Arc::into_raw(cloned) as *const (), &VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        // SAFETY: `data` is a valid `Arc<AtomicUsize>` pointer created by `Arc::into_raw`.
        let arc = unsafe { Arc::from_raw(data as *const AtomicUsize) };
        arc.fetch_add(1, Ordering::SeqCst);
        // arc is dropped here (consumed)
    }

    unsafe fn wake_by_ref(data: *const ()) {
        // SAFETY: `data` is a valid `Arc<AtomicUsize>` pointer created by `Arc::into_raw`.
        let arc = unsafe { Arc::from_raw(data as *const AtomicUsize) };
        arc.fetch_add(1, Ordering::SeqCst);
        let _ = Arc::into_raw(arc); // don't drop
    }

    unsafe fn drop_waker(data: *const ()) {
        // SAFETY: `data` is a valid `Arc<AtomicUsize>` pointer created by `Arc::into_raw`.
        unsafe { drop(Arc::from_raw(data as *const AtomicUsize)) };
    }

    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop_waker);

    let waker = unsafe { Waker::from_raw(RawWaker::new(data, &VTABLE)) };
    (waker, counter)
}
