//! A sleeping lock.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::panic::Location;
use core::sync::atomic::{AtomicBool, Ordering};

use super::level::{Ctx, Level, LockBefore, levels};
use super::lockdep::{self, Kind};
use super::wait::WaitQueue;

/// A lock at level `L` whose waiters sleep. `L` must come before
/// [`levels::MaySleep`], so a `Mutex` is never taken under a spin lock.
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

impl<L: Level + LockBefore<levels::MaySleep>, T> Mutex<L, T> {
    /// An unlocked mutex around `value`.
    pub(crate) const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            waiters: WaitQueue::new(),
            value: UnsafeCell::new(value),
            _level: PhantomData,
        }
    }

    /// Takes the lock, sleeping until it is free.
    #[track_caller]
    pub(crate) fn lock<'a, P>(&'a self, ctx: &'a mut Ctx<'_, P>) -> MutexGuard<'a, L, T>
    where
        P: LockBefore<L> + LockBefore<levels::MaySleep>,
    {
        lockdep::acquire::<L>(Kind::Sleeping, self.addr(), Location::caller());
        self.waiters.block_until(ctx, || self.try_acquire());
        MutexGuard {
            mutex: self,
            _not_send: PhantomData,
        }
    }

    /// Takes the lock and returns a token for its level, for taking later
    /// locks while holding it.
    #[track_caller]
    pub(crate) fn lock_and<'a, P>(
        &'a self,
        ctx: &'a mut Ctx<'_, P>,
    ) -> (MutexGuard<'a, L, T>, Ctx<'a, L>)
    where
        P: LockBefore<L> + LockBefore<levels::MaySleep>,
    {
        lockdep::acquire::<L>(Kind::Sleeping, self.addr(), Location::caller());
        self.waiters.block_until(ctx, || self.try_acquire());
        (
            MutexGuard {
                mutex: self,
                _not_send: PhantomData,
            },
            Ctx::new(),
        )
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
pub(crate) struct MutexGuard<'a, L: Level + LockBefore<levels::MaySleep>, T> {
    mutex: &'a Mutex<L, T>,
    _not_send: PhantomData<*const ()>,
}

impl<L: Level + LockBefore<levels::MaySleep>, T> Deref for MutexGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<L: Level + LockBefore<levels::MaySleep>, T> DerefMut for MutexGuard<'_, L, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<L: Level + LockBefore<levels::MaySleep>, T> Drop for MutexGuard<'_, L, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        lockdep::release(Kind::Sleeping, self.mutex.addr());
        self.mutex.waiters.wake_one();
    }
}
