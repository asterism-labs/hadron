//! Integration tests for subsystem tracing.
//!
//! Verifies that every `ktrace_subsys!` arm compiles and can be invoked
//! without crashing, regardless of whether the cfg flags are enabled or
//! disabled.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

hadron_test::test_entry_point_with_init!();

#[test_case]
fn test_ktrace_subsys_mm() {
    hadron_kernel::ktrace_subsys!(mm, "test {}", 42);
}

#[test_case]
fn test_ktrace_subsys_vfs() {
    hadron_kernel::ktrace_subsys!(vfs, "test");
}

#[test_case]
fn test_ktrace_subsys_sched() {
    hadron_kernel::ktrace_subsys!(sched, "test");
}

#[test_case]
fn test_ktrace_subsys_pci() {
    hadron_kernel::ktrace_subsys!(pci, "test");
}

#[test_case]
fn test_ktrace_subsys_acpi() {
    hadron_kernel::ktrace_subsys!(acpi, "test");
}

#[test_case]
fn test_ktrace_subsys_irq() {
    hadron_kernel::ktrace_subsys!(irq, "test");
}

#[test_case]
fn test_ktrace_subsys_syscall() {
    hadron_kernel::ktrace_subsys!(syscall, "test");
}

#[test_case]
fn test_ktrace_subsys_drivers() {
    hadron_kernel::ktrace_subsys!(drivers, "test");
}
