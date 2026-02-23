//! Alt-instruction inline patching tests.

use hadron_ktest::kernel_test;

// ---------------------------------------------------------------------------
// AltInstrEntry layout test
// ---------------------------------------------------------------------------

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_alt_instr_entry_size() {
    assert_eq!(
        core::mem::size_of::<crate::arch::x86_64::alt_instr::AltInstrEntry>(),
        32,
        "AltInstrEntry must be exactly 32 bytes to match GAS layout"
    );
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_alt_instr_entry_alignment() {
    assert_eq!(
        core::mem::align_of::<crate::arch::x86_64::alt_instr::AltInstrEntry>(),
        8,
        "AltInstrEntry must be 8-byte aligned"
    );
}

// ---------------------------------------------------------------------------
// Patching applied test
// ---------------------------------------------------------------------------

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_alt_instr_apply_ran() {
    // After boot, alt_instr::apply() should have run without panic.
    // We verify the entries are accessible (may be empty if no alt_instr!
    // sites were compiled in).
    let entries = crate::arch::x86_64::alt_instr::alt_instr_entries();
    let _ = entries.len();
}
