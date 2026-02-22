//! Early boot smoke tests — verifies the test harness itself works.

use hadron_ktest::kernel_test;

#[kernel_test(stage = "early_boot")]
fn test_trivial_assertion() {
    assert_eq!(1, 1);
}

#[kernel_test(stage = "early_boot")]
fn test_serial_output() {
    hadron_ktest::serial_println!("hello from ktest");
}

#[kernel_test(stage = "early_boot")]
fn test_breakpoint_exception_recovers() {
    // Execute int3; the IDT handler should catch it and return.
    unsafe {
        core::arch::asm!("int3", options(nomem, nostack));
    }
}
