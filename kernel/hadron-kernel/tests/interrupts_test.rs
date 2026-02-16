//! Integration tests for interrupt handling.
//!
//! Tests that exception handlers work and serial output functions.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

hadron_test::test_entry_point_with_init!();

#[test_case]
fn breakpoint_exception_recovers() {
    // Execute int3; the IDT handler should catch it and return.
    unsafe {
        core::arch::asm!("int3", options(nomem, nostack));
    }
}

#[test_case]
fn serial_output_works() {
    hadron_test::serial_println!("hello from interrupts_test");
}
