//! A sleeping lock.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::panic::Location;
use core::sync::atomic::{AtomicBool, Ordering};

use super::level::{Level, levels};
use super::lockdep::{self, Kind};
use super::wait::WaitQueue;
use crate::sched;

/// A lock at level `L` whose waiters sleep. `L` must come before
/// [`levels::MaySleep`], so taking locks in level order never takes a
/// `Mutex` under a spin lock.
///
/// It is not fair: a thread that arrives as the lock is released can take
/// it ahead of a sleeping waiter.
pub(crate) struct Mutex<L, T> {
    locked: AtomicBool,
    waiters: WaitQueue,
    value: UnsafeCell<T>,
    _level: PhantomData<fn() -> L>,
}

// SAFETY: the lock gives one holder at a time access to the value, so
// sharing it only needs the value to be sendable between threads.
unsafe impl<L, T: Send> Sync for Mutex<L, T> {}

impl<L: Level, T> Mutex<L, T> {
    /// An unlocked mutex around `value`.
    pub(crate) const fn new(value: T) -> Self {
        const {
            assert!(
                L::RANK < levels::MaySleep::RANK,
                "a mutex's level must come before `MaySleep`"
            );
        }
        Self {
            locked: AtomicBool::new(false),
            waiters: WaitQueue::new(),
            value: UnsafeCell::new(value),
            _level: PhantomData,
        }
    }

    /// Takes the lock, sleeping until it is free.
    ///
    /// # Panics
    ///
    /// Panics if the thread may not sleep (see [`sched::might_sleep`]).
    /// With `LOCKDEP`, also panics if the thread holds a lock at the same
    /// or a later level.
    #[track_caller]
    pub(crate) fn lock(&self) -> MutexGuard<'_, L, T> {
        sched::might_sleep();
        lockdep::acquire::<L>(Kind::Sleeping, self.addr(), Location::caller());
        self.waiters.block_until(|| self.try_acquire());
        MutexGuard {
            mutex: self,
            _not_send: PhantomData,
        }
    }

    fn try_acquire(&self) -> bool {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn addr(&self) -> usize {
        core::ptr::from_ref(self).addr()
    }
}

/// Access to the value of a held [`Mutex`]. Dropping it releases the lock
/// and wakes one waiter.
///
/// It is not `Send`: the run-time checker records it on this thread.
pub(crate) struct MutexGuard<'a, L: Level, T> {
    mutex: &'a Mutex<L, T>,
    _not_send: PhantomData<*const ()>,
}

impl<L: Level, T> Deref for MutexGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<L: Level, T> DerefMut for MutexGuard<'_, L, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<L: Level, T> Drop for MutexGuard<'_, L, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        lockdep::release(Kind::Sleeping, self.mutex.addr());
        self.mutex.waiters.wake_one();
    }
}
