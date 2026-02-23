//! Alternative function patching framework.
//!
//! Provides the core types and macros for alt-function dispatch points.
//! At boot the kernel's `apply()` function iterates all [`AltFnEntry`]
//! records placed in the `.hadron_alt_fn` linker section and patches
//! each dispatch pointer to the highest-priority alternative whose
//! required CPU features are present.
//!
//! Individual dispatch points are declared with the [`alt_fn!`] macro,
//! which creates a static [`AltFnDispatch`] (initialised to the
//! baseline implementation) and registers one [`AltFnEntry`] per
//! alternative in the linker section.
//!
//! Cross-crate alternatives can be registered with
//! [`alt_fn_alternative!`] without access to the original `alt_fn!`
//! declaration site.

use core::sync::atomic::{AtomicPtr, Ordering};

use crate::cpu_features::CpuFeatures;

// ---------------------------------------------------------------------------
// AltFnEntry — one entry per alternative implementation
// ---------------------------------------------------------------------------

/// A single alternative-function registration entry.
///
/// Placed in the `.hadron_alt_fn` linker section by [`alt_fn!`] or
/// [`alt_fn_alternative!`].
#[repr(C)]
pub struct AltFnEntry {
    /// Pointer to the `AtomicPtr<()>` that dispatches calls.
    pub fn_ptr: *const AtomicPtr<()>,
    /// Replacement function pointer (cast to `*mut ()`).
    pub replacement: *const (),
    /// Required CPU features for this alternative.
    pub feature: CpuFeatures,
    /// Priority — higher value wins when multiple alternatives match.
    pub priority: u8,
}

// SAFETY: AltFnEntry contains raw pointers to statics that live for the
// entire kernel lifetime. It is only read (never mutated) after link time.
unsafe impl Send for AltFnEntry {}
unsafe impl Sync for AltFnEntry {}

// ---------------------------------------------------------------------------
// Linkset declaration
// ---------------------------------------------------------------------------

#[cfg(target_os = "none")]
hadron_linkset::declare_linkset! {
    /// Returns all alt-function entries from the `.hadron_alt_fn` linker section.
    pub fn alt_fn_entries() -> [AltFnEntry],
    section = "hadron_alt_fn"
}

/// Host stub — returns an empty slice when not running on the kernel target.
#[cfg(not(target_os = "none"))]
pub fn alt_fn_entries() -> &'static [AltFnEntry] {
    &[]
}

// ---------------------------------------------------------------------------
// AltFnDispatch — type-safe dispatch wrapper
// ---------------------------------------------------------------------------

/// A type-safe wrapper around `AtomicPtr<()>` that provides dispatch.
pub struct AltFnDispatch<F> {
    ptr: AtomicPtr<()>,
    _marker: core::marker::PhantomData<F>,
}

// SAFETY: The AtomicPtr is only written during single-threaded boot patching
// and read thereafter via Acquire loads.
unsafe impl<F> Send for AltFnDispatch<F> {}
unsafe impl<F> Sync for AltFnDispatch<F> {}

impl<F> AltFnDispatch<F> {
    /// Creates a new dispatch initialised to `baseline`.
    pub const fn new(baseline: F) -> Self
    where
        F: Copy,
    {
        // SAFETY: Function pointers and *mut () have the same size and
        // alignment on all supported targets.
        let ptr = unsafe { core::mem::transmute_copy::<F, *mut ()>(&baseline) };
        Self {
            ptr: AtomicPtr::new(ptr),
            _marker: core::marker::PhantomData,
        }
    }

    /// Returns a pointer to the inner `AtomicPtr<()>` for linkset entries.
    pub const fn as_atomic_ptr(&self) -> *const AtomicPtr<()> {
        &self.ptr
    }

    /// Loads the current dispatch pointer.
    #[inline(always)]
    pub fn load(&self) -> F
    where
        F: Copy,
    {
        let raw = self.ptr.load(Ordering::Acquire);
        // SAFETY: The pointer was either the original baseline or a
        // replacement stored by apply(), both valid function pointers of
        // type F.
        unsafe { core::mem::transmute_copy::<*mut (), F>(&raw) }
    }
}

// ---------------------------------------------------------------------------
// alt_fn! macro
// ---------------------------------------------------------------------------

/// Declares an alternative-function dispatch point.
///
/// Creates:
/// 1. A `#[doc(hidden)]` module containing the `AltFnDispatch` static.
/// 2. A public `unsafe fn` wrapper that loads and calls through the dispatch.
/// 3. A linkset entry for each alternative registering it for patching.
///
/// The module lives in the type namespace and the function in the value
/// namespace, so `kernel_memcpy(args)` calls the wrapper and
/// `kernel_memcpy::DISPATCH` accesses the static for linkset entries.
///
/// Each alternative can be annotated with `#[cfg(...)]` to conditionally
/// compile it.
///
/// # Example
///
/// ```ignore
/// alt_fn! {
///     pub fn kernel_memcpy(dst: *mut u8, src: *const u8, len: usize),
///     baseline = memcpy_baseline,
///     alternatives = [
///         #[cfg(hadron_kernel_fpu)]
///         (CpuFeatures::SSE2, 1, memcpy_sse2),
///         (CpuFeatures::ERMS, 2, memcpy_erms),
///     ]
/// }
/// ```
#[macro_export]
macro_rules! alt_fn {
    (
        $(#[$meta:meta])*
        $vis:vis fn $name:ident($($arg:ident : $ty:ty),* $(,)?) $(-> $ret:ty)?,
        baseline = $baseline:path,
        alternatives = [ $( $(#[$alt_meta:meta])* ($feature:expr, $priority:expr, $alt:path) ),* $(,)? ]
    ) => {
        #[doc(hidden)]
        #[allow(non_snake_case, unused_imports)]
        $vis mod $name {
            use super::*;

            /// The dispatch static for this alt-function.
            pub static DISPATCH: $crate::alt_fn::AltFnDispatch<
                unsafe fn($($ty),*) $(-> $ret)?
            > = $crate::alt_fn::AltFnDispatch::new(
                $baseline as unsafe fn($($ty),*) $(-> $ret)?
            );
        }

        $(#[$meta])*
        #[inline(always)]
        $vis unsafe fn $name($($arg: $ty),*) $(-> $ret)? {
            let __f = $name::DISPATCH.load();
            unsafe { __f($($arg),*) }
        }

        // Register each alternative in the linker section.
        $(
            $(#[$alt_meta])*
            #[cfg(target_os = "none")]
            const _: () = {
                #[used]
                #[unsafe(link_section = ".hadron_alt_fn")]
                static ENTRY: $crate::alt_fn::AltFnEntry =
                    $crate::alt_fn::AltFnEntry {
                        fn_ptr: $name::DISPATCH.as_atomic_ptr(),
                        replacement: $alt as *const (),
                        feature: $feature,
                        priority: $priority,
                    };
            };
        )*
    };
}

// ---------------------------------------------------------------------------
// alt_fn_alternative! macro — cross-crate alternative registration
// ---------------------------------------------------------------------------

/// Registers an alternative implementation for a dispatch point defined
/// elsewhere (possibly in another crate).
///
/// Unlike inline alternatives in [`alt_fn!`], this macro can be invoked
/// from any crate that depends on the one declaring the dispatch point.
///
/// # Example
///
/// ```ignore
/// hadron_core::alt_fn_alternative! {
///     dispatch = hadron_core::mem::dispatch::kernel_memcpy,
///     #[cfg(hadron_kernel_fpu)]
///     (CpuFeatures::SSE2, 1, memcpy_sse2)
/// }
/// ```
#[macro_export]
macro_rules! alt_fn_alternative {
    (
        dispatch = $dispatch_mod:path,
        $(#[$alt_meta:meta])*
        ($feature:expr, $priority:expr, $alt:path)
    ) => {
        $(#[$alt_meta])*
        #[cfg(target_os = "none")]
        const _: () = {
            #[used]
            #[unsafe(link_section = ".hadron_alt_fn")]
            static ENTRY: $crate::alt_fn::AltFnEntry =
                $crate::alt_fn::AltFnEntry {
                    fn_ptr: <$dispatch_mod>::DISPATCH.as_atomic_ptr(),
                    replacement: $alt as *const (),
                    feature: $feature,
                    priority: $priority,
                };
        };
    };
}
