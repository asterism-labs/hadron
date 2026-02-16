//! Synchronization primitives for the kernel.
//!
//! Provides [`SpinLock`], [`RwLock`], and [`LazyLock`] suitable for use in
//! `static` items and usable before any allocator or scheduler is available.

mod irq_spinlock;
mod lazy;
mod mutex;
mod rwlock;
mod spinlock;
pub mod waitqueue;

#[cfg(test)]
pub(crate) mod test_waker;

pub use irq_spinlock::{IrqSpinLock, IrqSpinLockGuard};
pub use lazy::LazyLock;
pub use mutex::{Mutex, MutexGuard, MutexLockFuture};
pub use rwlock::{RwLock, RwLockReadGuard, RwLockWriteGuard};
pub use spinlock::{SpinLock, SpinLockGuard};
pub use waitqueue::WaitQueue;
