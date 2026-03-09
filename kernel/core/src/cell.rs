//! A minimal `UnsafeCell` wrapper that opts into `Send + Sync`.
//!
//! Used for statics where synchronization is handled externally (e.g., Limine
//! boot requests that are written once by the bootloader before kernel entry).

use core::cell::UnsafeCell;

/// An `UnsafeCell` wrapper that implements `Send` and `Sync`.
///
/// # Safety
///
/// The caller must ensure all accesses are properly synchronised. This type
/// exists for cases where the compiler cannot prove safety but the programmer
/// can (e.g., data written once before any concurrent access).
#[repr(transparent)]
pub struct RacyCell<T>(UnsafeCell<T>);

// SAFETY: The user of `RacyCell` is responsible for ensuring proper
// synchronisation. `T: Send` is required because the data may move between
// threads.
unsafe impl<T: Send> Send for RacyCell<T> {}
// SAFETY: Same as above — the user guarantees no data races.
unsafe impl<T: Sync> Sync for RacyCell<T> {}

impl<T> RacyCell<T> {
    /// Creates a new `RacyCell` wrapping `value`.
    #[inline]
    pub const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    /// Returns a raw pointer to the underlying data.
    #[inline]
    pub const fn get(&self) -> *mut T {
        self.0.get()
    }

    /// Returns a mutable reference to the underlying data.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        self.0.get_mut()
    }
}
