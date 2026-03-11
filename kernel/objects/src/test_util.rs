//! Test utilities for hadron-objects.

use core::sync::atomic::{AtomicU32, Ordering};
use core::task::{RawWaker, RawWakerVTable, Waker};

use alloc::sync::Arc;

/// Creates a `Waker` that increments a shared counter each time `wake()` is called.
///
/// Returns `(waker, counter)` where `counter` can be read to verify wake count.
pub fn counting_waker() -> (Waker, Arc<AtomicU32>) {
    let counter = Arc::new(AtomicU32::new(0));
    let data = Arc::into_raw(counter.clone()) as *const ();
    // SAFETY: data is a valid Arc<AtomicU32> pointer, vtable functions handle it correctly.
    let waker = unsafe { Waker::from_raw(RawWaker::new(data, &VTABLE)) };
    (waker, counter)
}

static VTABLE: RawWakerVTable = RawWakerVTable::new(clone_fn, wake_fn, wake_by_ref_fn, drop_fn);

unsafe fn clone_fn(data: *const ()) -> RawWaker {
    let arc = unsafe { Arc::from_raw(data as *const AtomicU32) };
    let cloned = arc.clone();
    core::mem::forget(arc);
    RawWaker::new(Arc::into_raw(cloned) as *const (), &VTABLE)
}

unsafe fn wake_fn(data: *const ()) {
    let arc = unsafe { Arc::from_raw(data as *const AtomicU32) };
    arc.fetch_add(1, Ordering::Relaxed);
    // Arc is dropped here (consumed)
}

unsafe fn wake_by_ref_fn(data: *const ()) {
    let arc = unsafe { Arc::from_raw(data as *const AtomicU32) };
    arc.fetch_add(1, Ordering::Relaxed);
    core::mem::forget(arc); // Don't drop -- wake_by_ref doesn't consume
}

unsafe fn drop_fn(data: *const ()) {
    let _ = unsafe { Arc::from_raw(data as *const AtomicU32) };
}
