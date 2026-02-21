//! Integration tests for the kernel backtrace module.
//!
//! Exercises `panic_backtrace()` as a regular function (not via panic) to
//! validate frame-pointer stack walking and HKIF symbolication in QEMU.
//!
//! The backtrace system is initialized during boot via `init_from_embedded()`,
//! which reads the `.hadron_hkif` linker section populated by the two-pass build.

#![no_std]
#![no_main]
#![allow(missing_docs)] // integration test
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

hadron_test::test_entry_point_with_init!();

use alloc::string::String;

#[test_case]
fn test_backtrace_captures_frames() {
    let mut output = String::new();
    hadron_kernel::backtrace::panic_backtrace(&mut output);
    // Frame walking should capture at least one frame (this test function
    // plus the test runner are on the call stack with frame pointers).
    assert!(
        output.contains("#0:"),
        "Expected at least one frame in output, got: {output}"
    );
}

#[test_case]
fn test_backtrace_symbolicated() {
    // The two-pass build embeds HKIF data in the kernel binary, which is
    // loaded during boot by init_from_embedded(). Verify that backtraces
    // include symbolicated output (function name after " - ").
    let mut output = String::new();
    hadron_kernel::backtrace::panic_backtrace(&mut output);
    assert!(
        output.contains(" - "),
        "Expected symbolicated frames (containing ' - '), got: {output}"
    );
}

#[inline(never)]
fn nested_a(output: &mut String) {
    nested_b(output);
}

#[inline(never)]
fn nested_b(output: &mut String) {
    nested_c(output);
}

#[inline(never)]
fn nested_c(output: &mut String) {
    hadron_kernel::backtrace::panic_backtrace(output);
}

#[test_case]
fn test_backtrace_nested_calls() {
    // Call through 3 nested #[inline(never)] functions.
    // This produces at least 3 frames on the stack.
    let mut output = String::new();
    nested_a(&mut output);
    // Verify at least 3 frames: #0, #1, #2
    assert!(
        output.contains("#2:"),
        "Expected at least 3 frames for nested calls, got: {output}"
    );
}
