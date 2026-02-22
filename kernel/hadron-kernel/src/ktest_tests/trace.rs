//! Subsystem tracing tests — verify every `ktrace_subsys!` arm compiles
//! and can be invoked without crashing.

use hadron_ktest::kernel_test;

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_ktrace_subsys_mm() {
    crate::ktrace_subsys!(mm, "test {}", 42);
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_ktrace_subsys_vfs() {
    crate::ktrace_subsys!(vfs, "test");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_ktrace_subsys_sched() {
    crate::ktrace_subsys!(sched, "test");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_ktrace_subsys_pci() {
    crate::ktrace_subsys!(pci, "test");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_ktrace_subsys_acpi() {
    crate::ktrace_subsys!(acpi, "test");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_ktrace_subsys_irq() {
    crate::ktrace_subsys!(irq, "test");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_ktrace_subsys_syscall() {
    crate::ktrace_subsys!(syscall, "test");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_ktrace_subsys_drivers() {
    crate::ktrace_subsys!(drivers, "test");
}
