//! Locks, wait queues and other synchronization.
//!
//! Every lock has a level, declared in [`level`], and `lockdep` checks at
//! run time that locks are taken in level order.

mod level;
mod lockdep;
mod mutex;
mod spin;
mod wait;

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU8, Ordering};

pub(crate) use level::levels;
pub(crate) use lockdep::HeldLocks;
pub(crate) use mutex::Mutex;
pub(crate) use spin::{IrqSpinLock, IrqSpinLockGuard, Mask};
pub(crate) use wait::WaitQueue;

/// No value yet.
const EMPTY: u8 = 0;
/// One caller is writing the value.
const WRITING: u8 = 1;
/// The value is written and can be read.
const READY: u8 = 2;

/// A static that is written once at run time and read after that.
///
/// It is meant for statics, so it never drops its value.
pub(crate) struct WriteOnce<T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
}

// SAFETY: `init` lets exactly one caller write the value, and readers see it
// only after the `Release` store of `READY`. After that the value is shared,
// so `T` must be `Sync`, and it may be written on another CPU, so `Send`.
unsafe impl<T: Send + Sync> Sync for WriteOnce<T> {}

impl<T> WriteOnce<T> {
    /// An empty cell.
    pub(crate) const fn new() -> Self {
        Self {
            state: AtomicU8::new(EMPTY),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Stores `value` and returns a reference to it.
    ///
    /// # Panics
    ///
    /// Panics if the cell was already written or is being written.
    pub(crate) fn init(&self, value: T) -> &T {
        let won = self
            .state
            .compare_exchange(EMPTY, WRITING, Ordering::Acquire, Ordering::Relaxed)
            .is_ok();
        assert!(won, "WriteOnce written twice");
        // SAFETY: winning the exchange gives this call the only access to
        // the value until it stores `READY`.
        unsafe { (*self.value.get()).write(value) };
        self.state.store(READY, Ordering::Release);
        self.get().expect("the value was just written")
    }

    /// The value, or `None` before [`init`](Self::init) finishes.
    pub(crate) fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == READY {
            // SAFETY: `READY` means the value is written and nobody writes it
            // again.
            Some(unsafe { (*self.value.get()).assume_init_ref() })
        } else {
            None
        }
    }
}

ktest::ktest! {
    fn write_once_reads_back_after_init() {
        let cell = WriteOnce::new();
        assert!(cell.get().is_none());
        assert_eq!(*cell.init(7_u32), 7);
        assert_eq!(cell.get(), Some(&7));
    }
}
