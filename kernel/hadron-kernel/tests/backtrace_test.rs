//! Integration tests for the kernel backtrace module.
//!
//! Exercises `panic_backtrace()` as a regular function (not via panic) to
//! validate frame-pointer stack walking and HBTF symbolication in QEMU.

#![no_std]
#![no_main]
#![allow(missing_docs)] // integration test
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

hadron_test::test_entry_point_with_init!();

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// Build a synthetic HBTF blob with a single catch-all symbol.
///
/// The symbol is at address 0 with size 0, meaning it matches any offset
/// (size=0 skips the bounds check in `lookup_symbol`). Combined with
/// `kernel_virt_base = 0`, every captured return address resolves to this symbol.
fn build_synthetic_hbtf() -> &'static [u8] {
    let name = b"test_func\0";

    let sym_count: u32 = 1;
    let line_count: u32 = 0;
    let header_size: u32 = 32;
    let sym_entry_size: u32 = 20;
    let sym_offset = header_size;
    let line_offset = sym_offset + sym_entry_size;
    let strings_offset = line_offset; // no line entries
    let strings_size = name.len() as u32;

    let total = strings_offset as usize + strings_size as usize;
    let mut buf = Vec::with_capacity(total);

    // Header (32 bytes)
    buf.extend_from_slice(b"HBTF");
    buf.extend_from_slice(&1u32.to_le_bytes()); // version
    buf.extend_from_slice(&sym_count.to_le_bytes());
    buf.extend_from_slice(&sym_offset.to_le_bytes());
    buf.extend_from_slice(&line_count.to_le_bytes());
    buf.extend_from_slice(&line_offset.to_le_bytes());
    buf.extend_from_slice(&strings_offset.to_le_bytes());
    buf.extend_from_slice(&strings_size.to_le_bytes());

    // Symbol entry: addr=0, size=0 (catch-all), name_off=0, reserved=0
    buf.extend_from_slice(&0u64.to_le_bytes()); // addr
    buf.extend_from_slice(&0u32.to_le_bytes()); // size
    buf.extend_from_slice(&0u32.to_le_bytes()); // name_off
    buf.extend_from_slice(&0u32.to_le_bytes()); // reserved

    // String pool
    buf.extend_from_slice(name);

    assert_eq!(buf.len(), total);

    Box::leak(buf.into_boxed_slice())
}

// Tests are ordered deliberately: tests that don't call init() come first,
// then tests that call init() with synthetic HBTF data. The custom test
// framework runs tests in file order.

#[test_case]
fn test_backtrace_without_init() {
    // Before any explicit backtrace::init(), panic_backtrace should not crash.
    // The test_entry_point_with_init!() passes backtrace: None, so the
    // backtrace module's global state is uninitialized.
    let mut output = String::new();
    hadron_kernel::backtrace::panic_backtrace(&mut output);
    // Output starts with "Backtrace" regardless of whether frames are captured.
    assert!(
        output.contains("Backtrace"),
        "Expected 'Backtrace' in output, got: {output}"
    );
}

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
fn test_backtrace_with_synthetic_hbtf() {
    let hbtf_data = build_synthetic_hbtf();
    // kernel_virt_base = 0 means offset = raw address, which our catch-all
    // symbol (addr=0, size=0) will always match.
    hadron_kernel::backtrace::init(hbtf_data, 0);

    let mut output = String::new();
    hadron_kernel::backtrace::panic_backtrace(&mut output);

    assert!(
        output.contains("test_func"),
        "Expected 'test_func' in symbolicated output, got: {output}"
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
