#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

hadron_test::test_entry_point!();

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}

#[test_case]
fn serial_output() {
    hadron_test::serial_println!("hello from test");
}
