//! Synchronization primitives for the kernel.
//!
//! Provides [`SpinLock`], [`RwLock`], and [`LazyLock`] suitable for use in
//! `static` items and usable before any allocator or scheduler is available.
//! Also provides [`HeapWaitQueue`] for service-layer primitives that need
//! unbounded capacity.

mod condvar;
mod heap_waitqueue;
mod irq_spinlock;
mod lazy;
#[cfg(hadron_lockdep)]
pub mod lockdep;
mod mutex;
mod rwlock;
mod semaphore;
mod seqlock;
mod spinlock;
pub mod waitqueue;

pub(crate) mod loom_compat;

#[cfg(test)]
pub(crate) mod test_waker;

pub use condvar::Condvar;
pub use heap_waitqueue::{HeapWaitFuture, HeapWaitQueue};
pub use irq_spinlock::{IrqSpinLock, IrqSpinLockGuard};
pub use lazy::LazyLock;
pub use mutex::{Mutex, MutexGuard, MutexLockFuture};
pub use rwlock::{RwLock, RwLockReadGuard, RwLockWriteGuard};
pub use semaphore::{Semaphore, SemaphoreAcquireFuture, SemaphorePermit};
pub use seqlock::{SeqLock, SeqLockWriteGuard};
pub use spinlock::{SpinLock, SpinLockGuard};
pub use waitqueue::WaitQueue;
