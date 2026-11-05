//! Synchronization helpers that do not depend on the locking model.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::arch::x86_64::cpu;

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

/// A spin lock that masks interrupts on this CPU while it is held.
///
/// Masking interrupts keeps an interrupt handler from spinning on a lock
/// the code it interrupted already holds. It is the heap's lock until the
/// kernel's locking model exists, and is not meant for general use.
pub(crate) struct IrqSpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the lock gives one holder at a time access to the value, so
// sharing the lock only needs the value to be sendable between CPUs.
unsafe impl<T: Send> Sync for IrqSpinLock<T> {}

impl<T> IrqSpinLock<T> {
    /// An unlocked lock around `value`.
    pub(crate) const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Masks interrupts, then spins until the lock is free.
    pub(crate) fn lock(&self) -> IrqSpinLockGuard<'_, T> {
        let interrupts = cpu::save_and_disable_interrupts();
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        IrqSpinLockGuard {
            lock: self,
            interrupts,
        }
    }
}

/// Access to the value of a held [`IrqSpinLock`]. Dropping it releases the
/// lock and restores the interrupt flag.
pub(crate) struct IrqSpinLockGuard<'a, T> {
    lock: &'a IrqSpinLock<T>,
    interrupts: bool,
}

impl<T> Deref for IrqSpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for IrqSpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for IrqSpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        cpu::restore_interrupts(self.interrupts);
    }
}

ktest::ktest! {
    fn irq_spin_lock_releases_on_drop() {
        let lock = IrqSpinLock::new(1_u32);
        *lock.lock() += 1;
        assert_eq!(*lock.lock(), 2);
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
