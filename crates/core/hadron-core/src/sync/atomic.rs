//! Atomic types compatible with both `core::sync::atomic` and `loom`.
//!
//! Under normal builds the wrapper types are zero-cost newtypes around the
//! standard library atomics with `const fn` constructors.  Under `cfg(loom)`
//! they delegate to `loom::sync::atomic` so that the loom model checker can
//! instrument every access.
//!
//! # Modules
//!
//! - Root types (`AtomicBool`, `AtomicU32`, …) — use these in sync primitives
//!   that must be loom-testable.  Constructors are `const` only on non-loom
//!   builds.
//! - [`const_only`] — types that are *always* backed by `core::sync::atomic`,
//!   with unconditionally `const` constructors.  Use these for infrastructure
//!   that is never loom-tested (lockdep, stress counters).
//!
//! # Free functions
//!
//! [`fence`] and [`compiler_fence`] mirror `core::sync::atomic::fence` /
//! `compiler_fence` but dispatch through loom when appropriate.

use core::sync::atomic as core_atomic;

#[cfg(loom)]
use loom::sync::atomic as loom_atomic;

#[cfg(not(loom))]
pub use core_atomic::Ordering;
#[cfg(loom)]
pub use loom_atomic::Ordering;

// ---------------------------------------------------------------------------
// Shared method bodies — invoked by both `define_atomic!` arms
// ---------------------------------------------------------------------------

macro_rules! define_atomic {
    // -- primary (loom-aware) integer type ------------------------------------
    (@primary $name:ident, $core_inner:ident, $ty:ty) => {
        /// Loom-compatible atomic wrapper.
        #[derive(Debug)]
        pub struct $name(
            #[cfg(not(loom))] core_atomic::$core_inner,
            #[cfg(loom)] loom_atomic::$core_inner,
        );

        // SAFETY: The inner type is already Send + Sync.
        unsafe impl Send for $name {}
        // SAFETY: The inner type is already Send + Sync.
        unsafe impl Sync for $name {}

        impl $name {
            maybe_const_fn! {
                /// Creates a new atomic value.
                pub fn new(value: $ty) -> Self {
                    #[cfg(not(loom))]
                    { Self(core_atomic::$core_inner::new(value)) }
                    #[cfg(loom)]
                    { Self(loom_atomic::$core_inner::new(value)) }
                }
            }

            define_atomic!(@common_methods $ty);
            define_atomic!(@numeric_methods $ty);
        }
    };

    // -- primary (loom-aware) bool type --------------------------------------
    (@primary_bool $name:ident, $core_inner:ident) => {
        /// Loom-compatible atomic bool wrapper.
        #[derive(Debug)]
        pub struct $name(
            #[cfg(not(loom))] core_atomic::$core_inner,
            #[cfg(loom)] loom_atomic::$core_inner,
        );

        // SAFETY: The inner type is already Send + Sync.
        unsafe impl Send for $name {}
        // SAFETY: The inner type is already Send + Sync.
        unsafe impl Sync for $name {}

        impl $name {
            maybe_const_fn! {
                /// Creates a new atomic bool.
                pub fn new(value: bool) -> Self {
                    #[cfg(not(loom))]
                    { Self(core_atomic::$core_inner::new(value)) }
                    #[cfg(loom)]
                    { Self(loom_atomic::$core_inner::new(value)) }
                }
            }

            define_atomic!(@common_methods bool);
            define_atomic!(@bool_methods);
        }
    };

    // -- const-only integer type (never loom-instrumented) -------------------
    (@const_only $name:ident, $core_inner:ident, $ty:ty) => {
        /// Atomic type backed unconditionally by `core::sync::atomic`.
        ///
        /// Constructors are always `const`.  Not loom-instrumented.
        #[derive(Debug)]
        pub struct $name(core_atomic::$core_inner);

        // SAFETY: The inner type is already Send + Sync.
        unsafe impl Send for $name {}
        // SAFETY: The inner type is already Send + Sync.
        unsafe impl Sync for $name {}

        impl $name {
            /// Creates a new atomic value (always `const`).
            pub const fn new(value: $ty) -> Self {
                Self(core_atomic::$core_inner::new(value))
            }

            define_atomic!(@common_methods $ty);
            define_atomic!(@numeric_methods $ty);
        }
    };

    // -- const-only bool type (never loom-instrumented) ----------------------
    (@const_only_bool $name:ident, $core_inner:ident) => {
        /// Atomic bool backed unconditionally by `core::sync::atomic`.
        ///
        /// Constructors are always `const`.  Not loom-instrumented.
        #[derive(Debug)]
        pub struct $name(core_atomic::$core_inner);

        // SAFETY: The inner type is already Send + Sync.
        unsafe impl Send for $name {}
        // SAFETY: The inner type is already Send + Sync.
        unsafe impl Sync for $name {}

        impl $name {
            /// Creates a new atomic bool (always `const`).
            pub const fn new(value: bool) -> Self {
                Self(core_atomic::$core_inner::new(value))
            }

            define_atomic!(@common_methods bool);
            define_atomic!(@bool_methods);
        }
    };

    // -- common methods (all atomic types) -----------------------------------
    (@common_methods $ty:ty) => {
        /// Loads a value from the atomic.
        #[inline]
        pub fn load(&self, order: Ordering) -> $ty {
            self.0.load(order)
        }

        /// Stores a value into the atomic.
        #[inline]
        pub fn store(&self, value: $ty, order: Ordering) {
            self.0.store(value, order);
        }

        /// Stores a value, returning the previous value.
        #[inline]
        pub fn swap(&self, value: $ty, order: Ordering) -> $ty {
            self.0.swap(value, order)
        }

        /// Stores a value if the current value equals `current`.
        #[inline]
        pub fn compare_exchange(
            &self,
            current: $ty,
            new: $ty,
            success: Ordering,
            failure: Ordering,
        ) -> Result<$ty, $ty> {
            self.0.compare_exchange(current, new, success, failure)
        }

        /// Weak version of [`compare_exchange`](Self::compare_exchange).
        #[inline]
        pub fn compare_exchange_weak(
            &self,
            current: $ty,
            new: $ty,
            success: Ordering,
            failure: Ordering,
        ) -> Result<$ty, $ty> {
            self.0.compare_exchange_weak(current, new, success, failure)
        }
    };

    // -- numeric methods (integer atomics only) ------------------------------
    (@numeric_methods $ty:ty) => {
        /// Adds to the current value, returning the previous value.
        #[inline]
        pub fn fetch_add(&self, value: $ty, order: Ordering) -> $ty {
            self.0.fetch_add(value, order)
        }

        /// Subtracts from the current value, returning the previous value.
        #[inline]
        pub fn fetch_sub(&self, value: $ty, order: Ordering) -> $ty {
            self.0.fetch_sub(value, order)
        }

        /// Bitwise OR with the current value, returning the previous value.
        #[inline]
        pub fn fetch_or(&self, value: $ty, order: Ordering) -> $ty {
            self.0.fetch_or(value, order)
        }

        /// Bitwise AND with the current value, returning the previous value.
        #[inline]
        pub fn fetch_and(&self, value: $ty, order: Ordering) -> $ty {
            self.0.fetch_and(value, order)
        }

        /// Bitwise XOR with the current value, returning the previous value.
        #[inline]
        pub fn fetch_xor(&self, value: $ty, order: Ordering) -> $ty {
            self.0.fetch_xor(value, order)
        }
    };

    // -- bool-specific methods (fetch_and/or/xor but NOT add/sub) -----------
    (@bool_methods) => {
        /// Logical OR with the current value, returning the previous value.
        #[inline]
        pub fn fetch_or(&self, value: bool, order: Ordering) -> bool {
            self.0.fetch_or(value, order)
        }

        /// Logical AND with the current value, returning the previous value.
        #[inline]
        pub fn fetch_and(&self, value: bool, order: Ordering) -> bool {
            self.0.fetch_and(value, order)
        }

        /// Logical XOR with the current value, returning the previous value.
        #[inline]
        pub fn fetch_xor(&self, value: bool, order: Ordering) -> bool {
            self.0.fetch_xor(value, order)
        }
    };
}

// ---------------------------------------------------------------------------
// Primary (loom-aware) types
// ---------------------------------------------------------------------------

define_atomic!(@primary_bool AtomicBool, AtomicBool);
define_atomic!(@primary AtomicU8, AtomicU8, u8);
define_atomic!(@primary AtomicU16, AtomicU16, u16);
define_atomic!(@primary AtomicU32, AtomicU32, u32);
define_atomic!(@primary AtomicU64, AtomicU64, u64);
define_atomic!(@primary AtomicUsize, AtomicUsize, usize);
define_atomic!(@primary AtomicI8, AtomicI8, i8);
define_atomic!(@primary AtomicI16, AtomicI16, i16);
define_atomic!(@primary AtomicI32, AtomicI32, i32);
define_atomic!(@primary AtomicI64, AtomicI64, i64);
define_atomic!(@primary AtomicIsize, AtomicIsize, isize);

// ---------------------------------------------------------------------------
// AtomicPtr<T> — uses the real backend type to preserve provenance
// ---------------------------------------------------------------------------

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
    maybe_const_fn! {
        /// Creates a new atomic pointer.
        pub fn new(ptr: *mut T) -> Self {
            #[cfg(not(loom))]
            { Self(core_atomic::AtomicPtr::new(ptr)) }
            #[cfg(loom)]
            { Self(loom_atomic::AtomicPtr::new(ptr)) }
        }
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

// ---------------------------------------------------------------------------
// Free functions: fence / compiler_fence
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// const_only — types that are never loom-instrumented
// ---------------------------------------------------------------------------

/// Atomic types backed unconditionally by `core::sync::atomic`.
///
/// These have always-`const` constructors and are suitable for
/// infrastructure that is never loom-tested (lockdep counters, stress
/// testing infrastructure, etc.).
pub mod const_only {
    use core::sync::atomic as core_atomic;

    // Re-export Ordering so callers can do `use const_only::*`.
    pub use core_atomic::Ordering;

    define_atomic!(@const_only_bool AtomicBool, AtomicBool);
    define_atomic!(@const_only AtomicU8, AtomicU8, u8);
    define_atomic!(@const_only AtomicU16, AtomicU16, u16);
    define_atomic!(@const_only AtomicU32, AtomicU32, u32);
    define_atomic!(@const_only AtomicU64, AtomicU64, u64);
    define_atomic!(@const_only AtomicUsize, AtomicUsize, usize);
    define_atomic!(@const_only AtomicI8, AtomicI8, i8);
    define_atomic!(@const_only AtomicI16, AtomicI16, i16);
    define_atomic!(@const_only AtomicI32, AtomicI32, i32);
    define_atomic!(@const_only AtomicI64, AtomicI64, i64);
    define_atomic!(@const_only AtomicIsize, AtomicIsize, isize);

    /// Atomic pointer backed unconditionally by `core::sync::atomic`.
    ///
    /// Always-`const` constructor, not loom-instrumented.
    #[derive(Debug)]
    pub struct AtomicPtr<T>(core_atomic::AtomicPtr<T>);

    // SAFETY: AtomicPtr is Send+Sync when T: Sync (matching core).
    unsafe impl<T> Send for AtomicPtr<T> {}
    // SAFETY: AtomicPtr is Send+Sync when T: Sync (matching core).
    unsafe impl<T> Sync for AtomicPtr<T> {}

    impl<T> AtomicPtr<T> {
        /// Creates a new atomic pointer (always `const`).
        pub const fn new(ptr: *mut T) -> Self {
            Self(core_atomic::AtomicPtr::new(ptr))
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
}
