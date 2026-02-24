//! Alternative instruction boot-time binary patching.
//!
//! The framework types and macros live in [`hadron_core::alt_instr`].
//! This module provides the kernel-side `apply()` function that
//! patches instruction bytes in `.text` at boot using detected CPU
//! features.

use hadron_core::sync::atomic::Ordering;

pub use hadron_core::alt_instr::{AltInstrEntry, alt_instr_entries};

use super::cpuid;
use super::registers::control::{Cr0, Cr0Flags};

// ---------------------------------------------------------------------------
// Boot-time patching
// ---------------------------------------------------------------------------

/// Patches instruction bytes in `.text` for the running CPU's feature set.
///
/// For each unique `instr_site`, selects the [`AltInstrEntry`] with the
/// highest `priority` whose `feature` flags are all present, copies the
/// replacement bytes over the original instructions, and NOP-pads any
/// remaining bytes.
///
/// # Safety
///
/// Must be called exactly once on the BSP, after [`cpuid::init()`] and
/// before any code that executes an `alt_instr!`-patched instruction.
/// Interrupts must be disabled.
pub unsafe fn apply() {
    let features = cpuid::cpu_features();
    let entries = alt_instr_entries();

    if entries.is_empty() {
        crate::kinfo!("alt-instr: no entries to patch");
        return;
    }

    let mut patched = 0usize;

    // Temporarily clear CR0.WP to write to .text (r-x segment).
    let cr0 = Cr0::read();
    unsafe { Cr0::write(cr0 & !Cr0Flags::WRITE_PROTECT) };

    for entry in entries {
        if !features.contains(entry.feature) {
            continue;
        }

        // Skip if dominated by a higher-priority alternative for the same site.
        let dominated = entries.iter().any(|other| {
            core::ptr::eq(other.instr_site, entry.instr_site)
                && other.priority > entry.priority
                && features.contains(other.feature)
        });

        if dominated {
            continue;
        }

        let dst = entry.instr_site as *mut u8;
        let repl_len = entry.repl_len as usize;
        let orig_len = entry.orig_len as usize;

        // Copy replacement bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(entry.replacement, dst, repl_len);
        }

        // NOP-pad remainder.
        let pad = orig_len.saturating_sub(repl_len);
        if pad > 0 {
            unsafe {
                core::ptr::write_bytes(dst.add(repl_len), 0x90, pad);
            }
        }

        patched += 1;
    }

    // Restore write protection.
    unsafe { Cr0::write(cr0) };
    hadron_core::sync::atomic::fence(Ordering::SeqCst);

    crate::kinfo!(
        "alt-instr: patched {} sites ({} entries, features={:?})",
        patched,
        entries.len(),
        features,
    );
}
