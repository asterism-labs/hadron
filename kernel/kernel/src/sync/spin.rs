//! Ticket spin locks.
//!
//! A [`SpinLock`] turns preemption off while held. An [`IrqSpinLock`] also
//! masks interrupts, so interrupt handlers may take it too. Both hand the
//! lock out in the order callers arrived, and waiters spin on a plain load.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::panic::Location;
use core::sync::atomic::{AtomicU16, Ordering};

use super::level::{Level, levels};
use super::lockdep::{self, Kind};
use crate::percpu::{IrqGuard, PreemptGuard};

/// What a spin lock keeps off while it is held.
pub(crate) trait Mask: Sized {
    /// Turns it off on this CPU.
    fn new() -> Self;

    /// A guard for a mask another guard set up and handed over.
    ///
    /// # Safety
    ///
    /// The per-CPU counts must already count this guard: another guard was
    /// made and forgotten, as when a lock is held across a context switch.
    unsafe fn assume() -> Self;
}

/// The lock word: the next ticket to hand out and the ticket being served.
struct Ticket {
    next: AtomicU16,
    serving: AtomicU16,
}

impl Ticket {
    const fn new() -> Self {
        Self {
            next: AtomicU16::new(0),
            serving: AtomicU16::new(0),
        }
    }

    fn lock(&self) {
        let ticket = self.next.fetch_add(1, Ordering::Relaxed);
        while self.serving.load(Ordering::Acquire) != ticket {
            core::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        // Only the holder writes `serving`, so a plain load and store does.
        let next = self.serving.load(Ordering::Relaxed).wrapping_add(1);
        self.serving.store(next, Ordering::Release);
    }
}

/// A spin lock at level `L` around a `T`. `M` is what stays off while it
/// is held.
pub(crate) struct SpinLock<L, T, M = PreemptGuard> {
    ticket: Ticket,
    value: UnsafeCell<T>,
    _level: PhantomData<fn() -> L>,
    _mask: PhantomData<fn() -> M>,
}

/// A spin lock that also masks interrupts.
pub(crate) type IrqSpinLock<L, T> = SpinLock<L, T, IrqGuard>;

/// The guard of an [`IrqSpinLock`].
pub(crate) type IrqSpinLockGuard<'a, L, T> = SpinLockGuard<'a, L, T, IrqGuard>;

// SAFETY: the lock gives one holder at a time access to the value, so
// sharing the lock only needs the value to be sendable between CPUs.
unsafe impl<L, T: Send, M> Sync for SpinLock<L, T, M> {}

impl<L: Level, T, M: Mask> SpinLock<L, T, M> {
    /// An unlocked lock around `value`.
    pub(crate) const fn new(value: T) -> Self {
        const {
            assert!(
                L::RANK > levels::MaySleep::RANK,
                "a spin lock's level must come after `MaySleep`"
            );
        }
        Self {
            ticket: Ticket::new(),
            value: UnsafeCell::new(value),
            _level: PhantomData,
            _mask: PhantomData,
        }
    }

    /// Takes the lock, spinning until it is free.
    ///
    /// # Panics
    ///
    /// With `LOCKDEP`, panics if this CPU or thread holds a lock at the
    /// same or a later level.
    #[track_caller]
    pub(crate) fn lock(&self) -> SpinLockGuard<'_, L, T, M> {
        let mask = M::new();
        lockdep::acquire::<L>(Kind::Spin, self.addr(), Location::caller());
        self.ticket.lock();
        SpinLockGuard { lock: self, mask }
    }

    /// A guard for the lock, which this CPU already holds through a guard
    /// that was forgotten.
    ///
    /// # Safety
    ///
    /// This CPU must hold the lock, and the forgotten guard must never be
    /// dropped.
    pub(crate) unsafe fn assume_locked(&self) -> SpinLockGuard<'_, L, T, M> {
        SpinLockGuard {
            lock: self,
            // SAFETY: the forgotten guard's mask is still counted, and the
            // caller hands it to this guard.
            mask: unsafe { M::assume() },
        }
    }

    fn addr(&self) -> usize {
        core::ptr::from_ref(self).addr()
    }
}

/// Access to the value of a held [`SpinLock`]. Dropping it releases the
/// lock, then drops the mask.
pub(crate) struct SpinLockGuard<'a, L: Level, T, M: Mask> {
    lock: &'a SpinLock<L, T, M>,
    /// Dropped after `drop` releases the lock.
    mask: M,
}

impl<L: Level, T, M: Mask> SpinLockGuard<'_, L, T, M> {
    /// What the guard keeps off, such as an [`IrqGuard`] that grants access
    /// to per-CPU data.
    pub(crate) fn mask(&self) -> &M {
        &self.mask
    }
}

impl<L: Level, T, M: Mask> Deref for SpinLockGuard<'_, L, T, M> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock.
        unsafe { &*self.lock.value.get() }
    }
}

impl<L: Level, T, M: Mask> DerefMut for SpinLockGuard<'_, L, T, M> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<L: Level, T, M: Mask> Drop for SpinLockGuard<'_, L, T, M> {
    fn drop(&mut self) {
        self.lock.ticket.unlock();
        lockdep::release(Kind::Spin, self.lock.addr());
    }
}

ktest::ktest! {
    fn spin_lock_releases_on_drop() {
        use super::levels::KtestSpin;

        static LOCK: IrqSpinLock<KtestSpin, u32> = IrqSpinLock::new(1);
        *LOCK.lock() += 1;
        assert_eq!(*LOCK.lock(), 2);
    }
}

ktest::ktest! {
    fn irq_spin_lock_masks_interrupts_while_held() {
        use super::levels::KtestSpin;
        use crate::arch::x86_64::cpu;

        static LOCK: IrqSpinLock<KtestSpin, ()> = IrqSpinLock::new(());
        let before = cpu::interrupts_enabled();
        let guard = LOCK.lock();
        assert!(!cpu::interrupts_enabled());
        drop(guard);
        assert_eq!(cpu::interrupts_enabled(), before);
    }
}
