//! Backtrace tests — frame-pointer stack walking and HKIF symbolication.

extern crate alloc;

use alloc::string::String;
use hadron_ktest::kernel_test;

#[kernel_test(stage = "early_boot")]
fn test_backtrace_captures_frames() {
    let mut output = String::new();
    crate::backtrace::panic_backtrace(&mut output);
    // Frame walking should capture at least one frame (this test function
    // plus the test runner are on the call stack with frame pointers).
    assert!(
        output.contains("#0:"),
        "Expected at least one frame in output, got: {output}"
    );
}

#[kernel_test(stage = "early_boot")]
fn test_backtrace_symbolicated() {
    let mut output = String::new();
    crate::backtrace::panic_backtrace(&mut output);
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
    crate::backtrace::panic_backtrace(output);
}

#[kernel_test(stage = "early_boot")]
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
