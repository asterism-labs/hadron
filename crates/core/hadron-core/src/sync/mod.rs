//! Synchronization primitives for the kernel.
//!
//! Provides [`SpinLock`], [`RwLock`], and [`LazyLock`] suitable for use in
//! `static` items and usable before any allocator or scheduler is available.
//! Also provides [`HeapWaitQueue`] for service-layer primitives that need
//! unbounded capacity.
//!
//! # Backend trait system
//!
//! All primitives are generic over a [`backend::Backend`] (or
//! [`backend::IrqBackend`]) trait, with concrete type aliases for
//! production use (e.g. `SpinLock<T> = SpinLockInner<T, CoreBackend>`).
//! This keeps `cfg(loom)` isolated to the [`backend`] module and enables
//! formal verification with Kani.
//!
//! # Loom testing
//!
//! All primitives can be tested under [loom](https://docs.rs/loom) (`just loom`)
//! by instantiating their `Inner` types with [`backend::LoomBackend`].
//!
//! ## What loom verifies
//!
//! - Lock protocol correctness (acquire/release ordering)
//! - Atomic state machine transitions (e.g. `LazyLock` init race)
//! - Waker management (no lost wakeups in `Mutex`, `Semaphore`)
//! - Absence of data races on protected data
//!
//! ## What loom does NOT verify
//!
//! - **Interrupt safety** — loom cannot model hardware interrupts;
//!   `IrqSpinLock` uses [`loom_mock`] thread-local stubs instead.
//! - **CPU-local isolation** — per-CPU storage is not modeled; lockdep
//!   nesting depth and stress PRNG state are tested on real hardware.
//! - **Preemption** — loom threads are cooperative; preemptive scheduling
//!   races are covered by ktest integration tests in QEMU.
//!
//! These hardware properties are tested by ktest integration tests
//! (`just test --kernel-only`) running on real (emulated) hardware.

pub mod atomic;
mod atomic_fn;
pub mod backend;
pub mod cell;
mod condvar;
mod heap_waitqueue;
mod irq_spinlock;
mod lazy;
#[cfg(hadron_lockdep)]
pub mod lockdep;
#[cfg(loom)]
mod loom_mock;
mod mutex;
mod rwlock;
mod semaphore;
mod seqlock;
mod spinlock;
#[cfg(hadron_lock_stress)]
pub mod stress;
pub mod waitqueue;

#[cfg(test)]
pub(crate) mod test_waker;

pub use atomic_fn::AtomicFn;
pub use condvar::{Condvar, CondvarInner};
pub use heap_waitqueue::{HeapWaitFuture, HeapWaitQueue, HeapWaitQueueInner};
pub use irq_spinlock::{IrqSpinLock, IrqSpinLockGuard, IrqSpinLockGuardInner, IrqSpinLockInner};
pub use lazy::{LazyLock, LazyLockInner};
pub use mutex::{Mutex, MutexGuard, MutexGuardInner, MutexInner, MutexLockFuture};
pub use rwlock::{
    RwLock, RwLockInner, RwLockReadGuard, RwLockReadGuardInner, RwLockWriteGuard,
    RwLockWriteGuardInner,
};
pub use semaphore::{Semaphore, SemaphoreAcquireFuture, SemaphoreInner, SemaphorePermit};
pub use seqlock::{SeqLock, SeqLockInner, SeqLockWriteGuard, SeqLockWriteGuardInner};
pub use spinlock::{SpinLock, SpinLockGuard, SpinLockGuardInner, SpinLockInner};
pub use waitqueue::{WaitQueue, WaitQueueInner};
