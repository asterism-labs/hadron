//! Atomic types compatible with both `core::sync::atomic` and `loom`.
//!
//! Under normal builds the types are re-exported directly from
//! `core::sync::atomic`.  Under `cfg(loom)` they come from
//! `loom::sync::atomic` so the model checker can instrument every access.
//!
//! [`AtomicPtr`] is a thin wrapper that preserves pointer provenance under
//! both backends.
//!
//! [`fence`] and [`compiler_fence`] mirror `core::sync::atomic::fence` /
//! `compiler_fence` but dispatch through loom when appropriate.

use core::sync::atomic as core_atomic;

#[cfg(loom)]
use loom::sync::atomic as loom_atomic;

// ─── Re-exports ───────────────────────────────────────────────────────

#[cfg(not(loom))]
pub use core_atomic::{
    AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8, AtomicU16,
    AtomicU32, AtomicU64, AtomicUsize, Ordering,
};

#[cfg(loom)]
pub use loom_atomic::{
    AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8, AtomicU16,
    AtomicU32, AtomicU64, AtomicUsize, Ordering,
};

// ─── AtomicPtr ────────────────────────────────────────────────────────

/// Loom-compatible atomic pointer.
///
/// Unlike the integer atomics this wraps the backend's own `AtomicPtr<T>`
/// directly so that pointer provenance is preserved.
#[derive(Debug)]
pub struct AtomicPtr<T>(
    #[cfg(not(loom))] core_atomic::AtomicPtr<T>,
    #[cfg(loom)] loom_atomic::AtomicPtr<T>,
);

// SAFETY: AtomicPtr is Send+Sync when T: Sync (matching core).
unsafe impl<T> Send for AtomicPtr<T> {}
// SAFETY: AtomicPtr is Send+Sync when T: Sync (matching core).
unsafe impl<T> Sync for AtomicPtr<T> {}

impl<T> AtomicPtr<T> {
    /// Creates a new atomic pointer.
    #[cfg(not(loom))]
    pub const fn new(ptr: *mut T) -> Self {
        Self(core_atomic::AtomicPtr::new(ptr))
    }

    /// Creates a new atomic pointer.
    #[cfg(loom)]
    pub fn new(ptr: *mut T) -> Self {
        Self(loom_atomic::AtomicPtr::new(ptr))
    }

    /// Loads the pointer value.
    #[inline]
    pub fn load(&self, order: Ordering) -> *mut T {
        self.0.load(order)
    }

    /// Stores a pointer value.
    #[inline]
    pub fn store(&self, ptr: *mut T, order: Ordering) {
        self.0.store(ptr, order);
    }

    /// Stores a pointer, returning the previous value.
    #[inline]
    pub fn swap(&self, ptr: *mut T, order: Ordering) -> *mut T {
        self.0.swap(ptr, order)
    }

    /// Stores a pointer if the current value equals `current`.
    #[inline]
    pub fn compare_exchange(
        &self,
        current: *mut T,
        new: *mut T,
        success: Ordering,
        failure: Ordering,
    ) -> Result<*mut T, *mut T> {
        self.0.compare_exchange(current, new, success, failure)
    }

    /// Weak version of [`compare_exchange`](Self::compare_exchange).
    #[inline]
    pub fn compare_exchange_weak(
        &self,
        current: *mut T,
        new: *mut T,
        success: Ordering,
        failure: Ordering,
    ) -> Result<*mut T, *mut T> {
        self.0.compare_exchange_weak(current, new, success, failure)
    }
}

// ─── Free functions ───────────────────────────────────────────────────

/// Memory fence — delegates to loom under `cfg(loom)`, otherwise
/// `core::sync::atomic::fence`.
#[inline]
pub fn fence(order: Ordering) {
    #[cfg(not(loom))]
    core_atomic::fence(order);
    #[cfg(loom)]
    loom_atomic::fence(order);
}

/// Compiler fence — always uses `core::sync::atomic::compiler_fence`.
///
/// Loom's model makes compiler fences redundant (every access is already
/// sequenced), so we unconditionally use the core version.
#[inline]
pub fn compiler_fence(order: Ordering) {
    core_atomic::compiler_fence(order);
}
