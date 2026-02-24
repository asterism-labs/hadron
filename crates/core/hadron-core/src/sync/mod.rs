//! Synchronization primitives for the kernel.
//!
//! Provides [`SpinLock`], [`RwLock`], and [`LazyLock`] suitable for use in
//! `static` items and usable before any allocator or scheduler is available.
//! Also provides [`HeapWaitQueue`] for service-layer primitives that need
//! unbounded capacity.
//!
//! # Loom testing
//!
//! All primitives are built on the [`atomic`] and [`cell`] compat layers so
//! they can be tested under [loom](https://docs.rs/loom) (`just loom`).
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

#[macro_use]
mod macros;
pub mod atomic;
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

/// Yields execution in a spin-wait loop.
///
/// Under normal builds this emits a processor spin-wait hint
/// (`core::hint::spin_loop()`). Under loom this yields to the
/// scheduler (`loom::thread::yield_now()`) so other threads can
/// make progress — loom's model checker cannot make forward progress
/// through `spin_loop()` hints since they are invisible to its
/// scheduler.
#[inline(always)]
fn spin_wait_hint() {
    #[cfg(not(loom))]
    core::hint::spin_loop();
    #[cfg(loom)]
    loom::thread::yield_now();
}

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
