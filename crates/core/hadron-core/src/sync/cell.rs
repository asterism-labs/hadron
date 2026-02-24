//! `UnsafeCell` wrappers compatible with both `core::cell` and `loom`.
//!
//! The root [`UnsafeCell`] is loom-aware: under `cfg(loom)` it delegates to
//! `loom::cell::UnsafeCell` so that loom's borrow checker can track every
//! access.  Only [`with`](UnsafeCell::with) and
//! [`with_mut`](UnsafeCell::with_mut) are exposed — there is no `get()`,
//! because a raw `get()` would bypass loom's instrumentation.
//!
//! [`const_only::UnsafeCell`] is always backed by `core::cell::UnsafeCell`
//! with an unconditionally `const` constructor and a raw `get()`.  Use it
//! only for infrastructure that is never loom-tested (lockdep, stress).

#[cfg(loom)]
use loom::cell as loom_cell;

/// Loom-compatible `UnsafeCell`.
///
/// Access the inner value only through [`with`](Self::with) and
/// [`with_mut`](Self::with_mut) to preserve loom's borrow tracking.
#[derive(Debug)]
pub struct UnsafeCell<T: ?Sized>(
    #[cfg(not(loom))] core::cell::UnsafeCell<T>,
    #[cfg(loom)] loom_cell::UnsafeCell<T>,
);

// SAFETY: UnsafeCell is Send if T: Send — matches core::cell::UnsafeCell.
unsafe impl<T: Send + ?Sized> Send for UnsafeCell<T> {}
// SAFETY: Sync is intentionally NOT implemented — same as core::cell::UnsafeCell.
// Sync is provided by the outer lock type.

impl<T> UnsafeCell<T> {
    maybe_const_fn! {
        /// Creates a new `UnsafeCell`.
        pub fn new(value: T) -> Self {
            #[cfg(not(loom))]
            { Self(core::cell::UnsafeCell::new(value)) }
            #[cfg(loom)]
            { Self(loom_cell::UnsafeCell::new(value)) }
        }
    }
}

impl<T: ?Sized> UnsafeCell<T> {
    /// Obtain a shared pointer to the inner value.
    #[cfg(not(loom))]
    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
        f(self.0.get() as *const T)
    }

    /// Obtain a shared pointer to the inner value.
    #[cfg(loom)]
    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
        self.0.with(f)
    }

    /// Obtain a mutable pointer to the inner value.
    #[cfg(not(loom))]
    #[inline]
    pub fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        f(self.0.get())
    }

    /// Obtain a mutable pointer to the inner value.
    #[cfg(loom)]
    #[inline]
    pub fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        self.0.with_mut(f)
    }
}

/// Atomic-free cell types that are never loom-instrumented.
///
/// These have unconditionally `const` constructors and expose a raw `get()`
/// method.  Only use them for infrastructure that is never loom-tested.
pub mod const_only {
    /// `UnsafeCell` backed unconditionally by `core::cell::UnsafeCell`.
    ///
    /// Always-`const` constructor, not loom-instrumented.  Exposes `get()`
    /// directly since loom tracking is not a concern.
    #[derive(Debug)]
    pub struct UnsafeCell<T: ?Sized>(core::cell::UnsafeCell<T>);

    // SAFETY: UnsafeCell is Send if T: Send — matches core::cell::UnsafeCell.
    unsafe impl<T: Send + ?Sized> Send for UnsafeCell<T> {}

    impl<T> UnsafeCell<T> {
        /// Creates a new `UnsafeCell` (always `const`).
        pub const fn new(value: T) -> Self {
            Self(core::cell::UnsafeCell::new(value))
        }
    }

    impl<T: ?Sized> UnsafeCell<T> {
        /// Returns a mutable pointer to the inner value.
        #[inline]
        pub fn get(&self) -> *mut T {
            self.0.get()
        }
    }
}
