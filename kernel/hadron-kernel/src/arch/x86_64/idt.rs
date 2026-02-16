//! IDT setup: exception handlers and hardware interrupt stubs.

use hadron_core::arch::x86_64::structures::idt::InterruptDescriptorTable;

use crate::sync::LazyLock;

use super::gdt::DOUBLE_FAULT_IST_INDEX;
use super::interrupts::{dispatch, handlers};

/// Static Interrupt Descriptor Table with all exception and hardware interrupt
/// handlers wired.
static IDT: LazyLock<InterruptDescriptorTable> = LazyLock::new(|| {
    let mut idt = InterruptDescriptorTable::new();

    // --- CPU Exception Handlers (vectors 0-31) ---

    idt.divide_error.set_handler(handlers::divide_error);
    idt.debug.set_handler(handlers::debug);
    idt.nmi.set_handler(handlers::nmi);
    idt.breakpoint.set_handler(handlers::breakpoint);
    idt.overflow.set_handler(handlers::overflow);
    idt.bound_range.set_handler(handlers::bound_range);
    idt.invalid_opcode.set_handler(handlers::invalid_opcode);
    idt.device_not_available
        .set_handler(handlers::device_not_available);
    idt.double_fault
        .set_diverging_handler_with_err_code(handlers::double_fault)
        .set_ist_index(DOUBLE_FAULT_IST_INDEX);
    idt.invalid_tss
        .set_handler_with_err_code(handlers::invalid_tss);
    idt.segment_not_present
        .set_handler_with_err_code(handlers::segment_not_present);
    idt.stack_segment_fault
        .set_handler_with_err_code(handlers::stack_segment_fault);
    idt.general_protection
        .set_handler_with_err_code(handlers::general_protection);
    idt.page_fault
        .set_handler_with_err_code(handlers::page_fault);
    idt.x87_floating_point
        .set_handler(handlers::x87_floating_point);
    idt.alignment_check
        .set_handler_with_err_code(handlers::alignment_check);
    idt.machine_check
        .set_diverging_handler(handlers::machine_check);
    idt.simd_floating_point
        .set_handler(handlers::simd_floating_point);
    idt.virtualization.set_handler(handlers::virtualization);
    idt.control_protection
        .set_handler_with_err_code(handlers::control_protection);
    idt.hypervisor_injection
        .set_handler(handlers::hypervisor_injection);
    idt.vmm_communication
        .set_handler_with_err_code(handlers::vmm_communication);
    idt.security_exception
        .set_handler_with_err_code(handlers::security_exception);

    // --- Hardware Interrupt Stubs (vectors 32-255) ---

    for (i, stub) in dispatch::STUBS.iter().enumerate() {
        idt.interrupts[i].set_handler(*stub);
    }

    idt
});

/// Loads the IDT into the CPU.
///
/// # Safety
///
/// Must be called after GDT initialization (CS must be valid).
pub unsafe fn init() {
    unsafe { IDT.load() };
    hadron_core::kdebug!("IDT initialized");
}
