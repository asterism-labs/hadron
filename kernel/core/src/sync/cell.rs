//! `UnsafeCell` wrapper providing closure-based access.
//!
//! This wrapper always uses `core::cell::UnsafeCell` internally with an
//! unconditionally `const` constructor.  Loom-aware cell access is handled
//! by the [`Backend`](super::backend::Backend) trait instead.
//!
//! Access the inner value only through [`with`](UnsafeCell::with) and
//! [`with_mut`](UnsafeCell::with_mut).

/// `UnsafeCell` with closure-based access.
///
/// Provides [`with`](Self::with) and [`with_mut`](Self::with_mut) instead
/// of a raw `get()` pointer, matching the API expected by sync primitives.
#[derive(Debug)]
pub struct UnsafeCell<T: ?Sized>(core::cell::UnsafeCell<T>);

// SAFETY: UnsafeCell is Send if T: Send — matches core::cell::UnsafeCell.
unsafe impl<T: Send + ?Sized> Send for UnsafeCell<T> {}
// SAFETY: Sync is intentionally NOT implemented — same as core::cell::UnsafeCell.
// Sync is provided by the outer lock type.

impl<T> UnsafeCell<T> {
    /// Creates a new `UnsafeCell`.
    pub const fn new(value: T) -> Self {
        Self(core::cell::UnsafeCell::new(value))
    }
}

impl<T: ?Sized> UnsafeCell<T> {
    /// Obtain a shared pointer to the inner value.
    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
        f(self.0.get() as *const T)
    }

    /// Obtain a mutable pointer to the inner value.
    #[inline]
    pub fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        f(self.0.get())
    }

    /// Returns a mutable pointer to the inner value.
    ///
    /// For use by infrastructure that is never loom-tested.
    #[inline]
    pub fn get(&self) -> *mut T {
        self.0.get()
    }
}
