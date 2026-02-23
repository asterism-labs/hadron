//! Alternative function patching engine.
//!
//! At boot the BSP detects CPU features via CPUID, then `apply()` iterates
//! all [`AltFnEntry`] records placed in the `.hadron_alt_fn` linker section
//! and patches each dispatch pointer to the highest-priority alternative
//! whose required features are present.
//!
//! Individual dispatch points are declared with the [`alt_fn!`] macro, which
//! creates a static `AtomicPtr` (initialised to the baseline implementation)
//! and registers one [`AltFnEntry`] per alternative in the linker section.

use core::sync::atomic::{AtomicPtr, Ordering};

use super::cpuid::{self, CpuFeatures};

// ---------------------------------------------------------------------------
// AltFnEntry — one entry per alternative implementation
// ---------------------------------------------------------------------------

/// A single alternative-function registration entry.
///
/// Placed in the `.hadron_alt_fn` linker section by [`alt_fn!`].
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

hadron_linkset::declare_linkset! {
    /// Returns all alt-function entries from the `.hadron_alt_fn` linker section.
    pub fn alt_fn_entries() -> [AltFnEntry],
    section = "hadron_alt_fn"
}

// ---------------------------------------------------------------------------
// Boot-time patching
// ---------------------------------------------------------------------------

/// Patches all alt-function dispatch pointers to the best available
/// implementation for the running CPU.
///
/// For each unique `fn_ptr` in the entry table, selects the entry with the
/// highest `priority` whose `feature` flags are all present, and stores
/// the `replacement` pointer.
///
/// # Safety
///
/// Must be called exactly once on the BSP, after [`cpuid::init()`] and
/// before any code that calls an `alt_fn!`-declared function.
pub unsafe fn apply() {
    let features = cpuid::cpu_features();
    let entries = alt_fn_entries();

    if entries.is_empty() {
        crate::kinfo!("alt-fn: no entries to patch");
        return;
    }

    let mut patched = 0usize;

    // For each entry, check if the current CPU supports the required
    // features and if this entry has a higher priority than what was
    // previously stored. We iterate all entries and always pick the
    // highest-priority match.
    //
    // Because entries for the same fn_ptr may appear in any order, we
    // do two passes: first collect the best match per fn_ptr, then
    // apply. With a small number of entries a simple O(n^2) scan is
    // fine.
    for entry in entries {
        if !features.contains(entry.feature) {
            continue;
        }

        // Check if another entry for the same fn_ptr with higher
        // priority was already considered. We scan all entries to find
        // the max priority for this fn_ptr.
        let dominated = entries.iter().any(|other| {
            core::ptr::eq(other.fn_ptr, entry.fn_ptr)
                && other.priority > entry.priority
                && features.contains(other.feature)
        });

        if dominated {
            continue;
        }

        // SAFETY: fn_ptr points to a valid AtomicPtr<()> static created
        // by the alt_fn! macro. replacement is a valid function pointer.
        unsafe {
            let dispatch = &*entry.fn_ptr;
            dispatch.store(entry.replacement as *mut (), Ordering::Relaxed);
        }
        patched += 1;
    }

    // Ensure all stores are visible before any dispatch call.
    core::sync::atomic::fence(Ordering::Release);

    crate::kinfo!(
        "alt-fn: patched {} dispatch points ({} entries, features={:?})",
        patched,
        entries.len(),
        features,
    );
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

            pub static DISPATCH: $crate::arch::x86_64::alt_fn::AltFnDispatch<
                unsafe fn($($ty),*) $(-> $ret)?
            > = $crate::arch::x86_64::alt_fn::AltFnDispatch::new(
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
            const _: () = {
                #[used]
                #[unsafe(link_section = ".hadron_alt_fn")]
                static ENTRY: $crate::arch::x86_64::alt_fn::AltFnEntry =
                    $crate::arch::x86_64::alt_fn::AltFnEntry {
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
