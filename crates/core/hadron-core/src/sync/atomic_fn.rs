//! Type-safe atomic function pointer.
//!
//! [`AtomicFn`] wraps [`AtomicPtr<()>`] with a phantom type parameter that
//! enforces the function signature. All `transmute` calls between `*mut ()`
//! and `fn(...)` are confined to the [`store`](AtomicFn::store) and
//! [`load`](AtomicFn::load) methods.
//!
//! # Usage
//!
//! ```ignore
//! static PRINT_FN: AtomicFn<fn(fmt::Arguments<'_>)> =
//!     AtomicFn::new(null_print);
//!
//! // Store a new function pointer (safe):
//! PRINT_FN.store(real_print);
//!
//! // Load and call:
//! PRINT_FN.load()(args);
//! ```

use core::marker::PhantomData;

use super::atomic::{AtomicPtr, Ordering};

/// Atomic storage for a function pointer of type `F`.
///
/// `F` must be a `fn(...)` type (which is `Copy + Send + Sync`).
/// The wrapper guarantees that only values of type `F` are stored
/// and loaded, confining the `transmute` to a single well-audited site.
pub struct AtomicFn<F: Copy> {
    ptr: AtomicPtr<()>,
    _marker: PhantomData<F>,
}

// SAFETY: Function pointers are inherently Send + Sync.
unsafe impl<F: Copy> Send for AtomicFn<F> {}
// SAFETY: All access is through AtomicPtr which is Sync.
unsafe impl<F: Copy> Sync for AtomicFn<F> {}

impl<F: Copy> AtomicFn<F> {
    /// Creates a new `AtomicFn` initialized with `f`.
    ///
    /// # Safety (compile-time)
    ///
    /// The caller must ensure `F` is a function pointer type (e.g.
    /// `fn(u32) -> bool`). This is not enforced by the type system but
    /// is an API contract — storing non-fn-pointer `Copy` types is unsound.
    pub const fn new(f: F) -> Self {
        // SAFETY: Function pointers and `*mut ()` have the same size and
        // representation on all Rust targets. This is a const-context
        // transmute confined to this single constructor.
        let ptr = unsafe { core::mem::transmute_copy(&f) };
        Self {
            ptr: AtomicPtr::new(ptr),
            _marker: PhantomData,
        }
    }

    /// Creates a new `AtomicFn` initialized to null.
    ///
    /// [`load_optional`](Self::load_optional) must be used to safely handle
    /// the null case. Calling [`load`](Self::load) on a null `AtomicFn` is
    /// undefined behavior.
    pub const fn null() -> Self {
        Self {
            ptr: AtomicPtr::new(core::ptr::null_mut()),
            _marker: PhantomData,
        }
    }

    /// Atomically stores a new function pointer.
    pub fn store(&self, f: F) {
        // SAFETY: Same transmute as `new` — fn pointer to `*mut ()`.
        let ptr = unsafe { core::mem::transmute_copy(&f) };
        self.ptr.store(ptr, Ordering::Release);
    }

    /// Atomically loads the function pointer.
    ///
    /// # Safety
    ///
    /// The stored pointer must be non-null (i.e., initialized via [`new`]
    /// or a prior [`store`] call). For nullable slots use
    /// [`load_optional`](Self::load_optional).
    #[inline]
    pub fn load(&self) -> F {
        let ptr = self.ptr.load(Ordering::Acquire);
        // SAFETY: The caller guarantees the pointer is non-null and was
        // stored as a valid `F` by `new` or `store`.
        unsafe { core::mem::transmute_copy(&ptr) }
    }

    /// Atomically loads the function pointer, returning `None` if null.
    #[inline]
    pub fn load_optional(&self) -> Option<F> {
        let ptr = self.ptr.load(Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            // SAFETY: Non-null pointers were stored as valid `F` by `store`.
            Some(unsafe { core::mem::transmute_copy(&ptr) })
        }
    }

    /// Atomically compare-exchange: if the current pointer is null, set it
    /// to `f`. Returns `Ok(())` on success, `Err(())` if already set.
    pub fn try_set(&self, f: F) -> Result<(), ()> {
        // SAFETY: Same transmute as `store`.
        let new_ptr: *mut () = unsafe { core::mem::transmute_copy(&f) };
        self.ptr
            .compare_exchange(
                core::ptr::null_mut(),
                new_ptr,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|_| ())
    }

    /// Atomically sets the pointer back to null.
    pub fn clear(&self) {
        self.ptr.store(core::ptr::null_mut(), Ordering::Release);
    }
}
