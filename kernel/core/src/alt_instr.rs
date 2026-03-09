//! Alternative instruction inline binary patching framework.
//!
//! Provides the core types and macros for inline instruction patching.
//! At boot the kernel's `apply()` function iterates all [`AltInstrEntry`]
//! records placed in the `.hadron_alt_instr` linker section and patches
//! instruction bytes in `.text` in-place.
//!
//! Unlike [`alt_fn!`](crate::alt_fn) (one atomic-load per call),
//! `alt_instr!` has **zero** runtime overhead after boot — the patched
//! instructions execute directly. Trade-off: the replacement must fit
//! within the original instruction's byte length (padded with NOPs if
//! shorter).
//!
//! # Usage
//!
//! The [`alt_instr!`] macro is designed to be used inside
//! `core::arch::asm!` or `core::arch::global_asm!` blocks. It expands
//! to a `concat!` of GAS directives that:
//!
//! 1. Emit the default instruction sequence in `.text`.
//! 2. Place replacement bytes in `.hadron_alt_instr_replacement`.
//! 3. Register an [`AltInstrEntry`] in `.hadron_alt_instr`.

use crate::cpu_features::CpuFeatures;

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

#[cfg(target_os = "none")]
hadron_linkset::declare_linkset! {
    /// Returns all alt-instruction entries from the `.hadron_alt_instr` linker section.
    pub fn alt_instr_entries() -> [AltInstrEntry],
    section = "hadron_alt_instr"
}

/// Host stub — returns an empty slice when not running on the kernel target.
#[cfg(not(target_os = "none"))]
pub fn alt_instr_entries() -> &'static [AltInstrEntry] {
    &[]
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
