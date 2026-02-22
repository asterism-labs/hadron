//! IDT setup: exception handlers and hardware interrupt stubs.

use crate::arch::x86_64::structures::idt::InterruptDescriptorTable;

use crate::sync::LazyLock;

use super::gdt::DOUBLE_FAULT_IST_INDEX;
use super::interrupts::{dispatch, exception_table::exception_table, handlers, timer_stub};

/// Static Interrupt Descriptor Table with all exception and hardware interrupt
/// handlers wired.
static IDT: LazyLock<InterruptDescriptorTable> = LazyLock::new(|| {
    let mut idt = InterruptDescriptorTable::new();

    // --- CPU Exception Handlers (vectors 0-31) ---
    //
    // The exception_table! macro type-checks each handler against the expected
    // signature at compile time. If a handler has the wrong type (e.g., a
    // non-diverging double fault handler), compilation fails with a clear
    // type mismatch error.

    exception_table! {
        idt = idt;
        divide_error          => plain(handlers::divide_error);
        debug                 => plain(handlers::debug);
        nmi                   => plain(handlers::nmi);
        breakpoint            => plain(handlers::breakpoint), dpl = 3;
        overflow              => plain(handlers::overflow);
        bound_range           => plain(handlers::bound_range);
        invalid_opcode        => plain(handlers::invalid_opcode);
        device_not_available  => plain(handlers::device_not_available);
        double_fault          => diverging_err(handlers::double_fault), ist = DOUBLE_FAULT_IST_INDEX;
        invalid_tss           => with_err_code(handlers::invalid_tss);
        segment_not_present   => with_err_code(handlers::segment_not_present);
        stack_segment_fault   => with_err_code(handlers::stack_segment_fault);
        general_protection    => with_err_code(handlers::general_protection);
        page_fault            => with_err_code(handlers::page_fault);
        x87_floating_point    => plain(handlers::x87_floating_point);
        alignment_check       => with_err_code(handlers::alignment_check);
        machine_check         => diverging(handlers::machine_check);
        simd_floating_point   => plain(handlers::simd_floating_point);
        virtualization        => plain(handlers::virtualization);
        control_protection    => with_err_code(handlers::control_protection);
        hypervisor_injection  => plain(handlers::hypervisor_injection);
        vmm_communication     => with_err_code(handlers::vmm_communication);
        security_exception    => with_err_code(handlers::security_exception);
    }

    // --- Hardware Interrupt Stubs (vectors 32-255) ---
    //
    // Each stub is a naked function that handles swapgs, register save/restore,
    // and dispatch. Installed via `set_naked_stub` for self-documenting intent.

    for (i, stub) in dispatch::STUBS.iter().enumerate() {
        // SAFETY: Each stub follows the hardware interrupt calling convention
        // (swapgs, scratch reg save/restore, dispatch call, iretq).
        unsafe { idt.interrupts[i].set_naked_stub(*stub) };
    }

    // Override vector 254 (LAPIC timer) with the custom preemption-aware
    // stub that saves user register state on ring-3 interrupts.
    // SAFETY: timer_preempt_stub follows the interrupt stub convention with
    // additional full-register-state save for userspace preemption.
    unsafe {
        idt.interrupts[dispatch::vectors::TIMER.table_index()]
            .set_naked_stub(timer_stub::timer_preempt_stub);
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
    crate::kdebug!("IDT initialized");
}
