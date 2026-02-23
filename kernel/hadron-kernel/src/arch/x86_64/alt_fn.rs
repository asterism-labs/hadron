//! Alternative function boot-time patching.
//!
//! The framework types and macros live in [`hadron_core::alt_fn`].
//! This module provides the kernel-side `apply()` function that
//! patches dispatch pointers at boot using detected CPU features.

use core::sync::atomic::Ordering;

pub use hadron_core::alt_fn::{AltFnDispatch, AltFnEntry, alt_fn_entries};

use super::cpuid;

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
