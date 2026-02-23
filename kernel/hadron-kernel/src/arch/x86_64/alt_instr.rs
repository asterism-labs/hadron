//! Alternative instruction inline binary patching engine.
//!
//! At boot the BSP detects CPU features via CPUID, then [`apply()`] iterates
//! all [`AltInstrEntry`] records placed in the `.hadron_alt_instr` linker
//! section and patches instruction bytes in `.text` in-place.
//!
//! Unlike [`alt_fn!`](super::alt_fn) (one atomic-load per call), `alt_instr!`
//! has **zero** runtime overhead after boot — the patched instructions execute
//! directly. Trade-off: the replacement must fit within the original
//! instruction's byte length (padded with NOPs if shorter).
//!
//! # Usage
//!
//! The [`alt_instr!`] macro is designed to be used inside `core::arch::asm!`
//! or `core::arch::global_asm!` blocks. It expands to a `concat!` of GAS
//! directives that:
//!
//! 1. Emit the default instruction sequence in `.text`.
//! 2. Place replacement bytes in `.hadron_alt_instr_replacement`.
//! 3. Register an [`AltInstrEntry`] in `.hadron_alt_instr`.

use core::sync::atomic::Ordering;

use super::cpuid::{self, CpuFeatures};
use super::registers::control::{Cr0, Cr0Flags};

// ---------------------------------------------------------------------------
// AltInstrEntry — metadata for one inline patch site
// ---------------------------------------------------------------------------

/// Metadata for one inline instruction patch site.
///
/// Placed in the `.hadron_alt_instr` linker section by [`alt_instr!`].
/// Total size: 32 bytes, naturally aligned with `#[repr(C)]`.
#[repr(C)]
pub struct AltInstrEntry {
    /// Pointer to the patch site in `.text`.
    pub instr_site: *const u8,
    /// Pointer to replacement bytes in `.hadron_alt_instr_replacement`.
    pub replacement: *const u8,
    /// Required CPU features for this alternative.
    pub feature: CpuFeatures,
    /// Length of the original instruction sequence.
    pub orig_len: u8,
    /// Length of the replacement sequence.
    pub repl_len: u8,
    /// Priority — higher value wins when multiple alternatives target the same site.
    pub priority: u8,
    /// Padding to 32 bytes.
    pub _pad: [u8; 5],
}

// SAFETY: AltInstrEntry contains raw pointers to statics that live for the
// entire kernel lifetime. It is only read (never mutated) after link time.
unsafe impl Send for AltInstrEntry {}
unsafe impl Sync for AltInstrEntry {}

// ---------------------------------------------------------------------------
// Linkset declaration
// ---------------------------------------------------------------------------

hadron_linkset::declare_linkset! {
    /// Returns all alt-instruction entries from the `.hadron_alt_instr` linker section.
    pub fn alt_instr_entries() -> [AltInstrEntry],
    section = "hadron_alt_instr"
}

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
    core::sync::atomic::fence(Ordering::SeqCst);

    crate::kinfo!(
        "alt-instr: patched {} sites ({} entries, features={:?})",
        patched,
        entries.len(),
        features,
    );
}

// ---------------------------------------------------------------------------
// alt_instr! macro — for use inside asm!() blocks
// ---------------------------------------------------------------------------

/// Generates inline alternative-instruction GAS directives.
///
/// Expands to a `concat!` string suitable for embedding in `asm!()` or
/// `global_asm!()`. The caller must provide a unique `$label` identifier
/// (Rust `asm!` has no `%=` equivalent).
///
/// # Arguments
///
/// - `$label` — Unique identifier for local labels.
/// - `default` — Default instruction(s) as a GAS string literal.
/// - `alternative` — Replacement instruction(s) as a GAS string literal.
/// - `feature_bits` — CPU feature bits as a string literal (use [`alt_instr_feature_bits!`]).
/// - `priority` — Priority value (higher wins).
///
/// # Example
///
/// ```ignore
/// core::arch::asm!(
///     alt_instr!(
///         my_fence,
///         default = "mfence",
///         alternative = "lfence",
///         feature_bits = alt_instr_feature_bits!(SSE2),
///         priority = 1,
///     ),
///     options(nostack, preserves_flags),
/// );
/// ```
#[macro_export]
macro_rules! alt_instr {
    (
        $label:ident,
        default = $default:expr,
        alternative = $alt:expr,
        feature_bits = $feature_bits:expr,
        priority = $priority:expr $(,)?
    ) => {
        concat!(
            // Default code in .text
            ".Lalt_orig_",
            stringify!($label),
            ":\n",
            $default,
            "\n",
            ".Lalt_orig_end_",
            stringify!($label),
            ":\n",
            // Replacement bytes in .hadron_alt_instr_replacement
            ".pushsection .hadron_alt_instr_replacement, \"a\"\n",
            ".Lalt_repl_",
            stringify!($label),
            ":\n",
            $alt,
            "\n",
            ".Lalt_repl_end_",
            stringify!($label),
            ":\n",
            ".popsection\n",
            // Metadata entry in .hadron_alt_instr (must match AltInstrEntry layout)
            ".pushsection .hadron_alt_instr, \"a\"\n",
            ".balign 8\n",
            ".quad .Lalt_orig_",
            stringify!($label),
            "\n", // instr_site
            ".quad .Lalt_repl_",
            stringify!($label),
            "\n", // replacement
            ".quad ",
            $feature_bits,
            "\n", // feature (u64)
            ".byte .Lalt_orig_end_",
            stringify!($label),
            " - .Lalt_orig_",
            stringify!($label),
            "\n", // orig_len
            ".byte .Lalt_repl_end_",
            stringify!($label),
            " - .Lalt_repl_",
            stringify!($label),
            "\n", // repl_len
            ".byte ",
            stringify!($priority),
            "\n",        // priority
            ".zero 5\n", // _pad
            ".popsection\n",
        )
    };
}

// ---------------------------------------------------------------------------
// alt_instr_feature_bits! — GAS-compatible feature bit string literals
// ---------------------------------------------------------------------------

/// Maps CPU feature names to their bit values as string literals for use
/// in GAS directives (which cannot evaluate Rust expressions).
#[macro_export]
macro_rules! alt_instr_feature_bits {
    (SSE2) => {
        "256"
    }; // 1 << 8
    (SSE3) => {
        "1"
    }; // 1 << 0
    (SSSE3) => {
        "2"
    }; // 1 << 1
    (SSE4_1) => {
        "4"
    }; // 1 << 2
    (SSE4_2) => {
        "8"
    }; // 1 << 3
    (POPCNT) => {
        "16"
    }; // 1 << 4
    (ERMS) => {
        "524288"
    }; // 1 << 19
    (AVX) => {
        "64"
    }; // 1 << 6
    (AVX2) => {
        "65536"
    }; // 1 << 16
}
