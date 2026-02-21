//! Loom compatibility shim.
//!
//! When compiled with `cfg(loom)`, re-exports loom's concurrency primitives.
//! Otherwise, re-exports the standard `core::sync::atomic` types and
//! `core::cell::UnsafeCell`.
//!
//! This allows sync primitives to be tested under loom's deterministic
//! scheduler without code changes.

// ---------------------------------------------------------------------------
// Loom mode
// ---------------------------------------------------------------------------

#[cfg(loom)]
pub(crate) use loom::cell::UnsafeCell;
#[cfg(loom)]
pub(crate) use loom::sync::atomic::{
    AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize,
};
#[cfg(loom)]
pub(crate) use loom::sync::atomic::Ordering;

// ---------------------------------------------------------------------------
// Normal mode
// ---------------------------------------------------------------------------

#[cfg(not(loom))]
pub(crate) use core::cell::UnsafeCell;
#[cfg(not(loom))]
pub(crate) use core::sync::atomic::{
    AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};
