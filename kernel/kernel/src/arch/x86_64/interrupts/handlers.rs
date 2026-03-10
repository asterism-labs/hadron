//! Default CPU exception handlers.
//!
//! All handlers use the `x86-interrupt` ABI. Most panic with the exception name
//! and stack frame. `debug` and `breakpoint` log and return for debugging.
//! User-mode fault handling is deferred until the process subsystem is
//! integrated.

// Handler names are self-documenting; suppress missing_docs for this module.
#![allow(missing_docs)]

use crate::arch::x86_64::structures::idt::InterruptStackFrame;

pub extern "x86-interrupt" fn divide_error(frame: InterruptStackFrame) {
    panic!("EXCEPTION: DIVIDE ERROR\n{:#?}", frame);
}

pub extern "x86-interrupt" fn debug(frame: InterruptStackFrame) {
    crate::kwarn!("exception", "EXCEPTION: DEBUG\n{:#?}", frame);
}

pub extern "x86-interrupt" fn nmi(_frame: InterruptStackFrame) {
    panic!("EXCEPTION: NON-MASKABLE INTERRUPT\n{:#?}", _frame);
}

pub extern "x86-interrupt" fn breakpoint(frame: InterruptStackFrame) {
    crate::kwarn!("exception", "EXCEPTION: BREAKPOINT\n{:#?}", frame);
}

pub extern "x86-interrupt" fn overflow(frame: InterruptStackFrame) {
    panic!("EXCEPTION: OVERFLOW\n{:#?}", frame);
}

pub extern "x86-interrupt" fn bound_range(frame: InterruptStackFrame) {
    panic!("EXCEPTION: BOUND RANGE EXCEEDED\n{:#?}", frame);
}

pub extern "x86-interrupt" fn invalid_opcode(frame: InterruptStackFrame) {
    panic!("EXCEPTION: INVALID OPCODE\n{:#?}", frame);
}

pub extern "x86-interrupt" fn device_not_available(frame: InterruptStackFrame) {
    panic!("EXCEPTION: DEVICE NOT AVAILABLE\n{:#?}", frame);
}

pub extern "x86-interrupt" fn double_fault(frame: InterruptStackFrame, error_code: u64) -> ! {
    panic!(
        "EXCEPTION: DOUBLE FAULT (error_code={})\n{:#?}",
        error_code, frame
    );
}

pub extern "x86-interrupt" fn invalid_tss(frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: INVALID TSS (error_code={:#x})\n{:#?}",
        error_code, frame
    );
}

pub extern "x86-interrupt" fn segment_not_present(frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: SEGMENT NOT PRESENT (error_code={:#x})\n{:#?}",
        error_code, frame
    );
}

pub extern "x86-interrupt" fn stack_segment_fault(frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: STACK-SEGMENT FAULT (error_code={:#x})\n{:#?}",
        error_code, frame
    );
}

pub extern "x86-interrupt" fn general_protection(frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT (error_code={:#x})\n{:#?}",
        error_code, frame
    );
}

pub extern "x86-interrupt" fn page_fault(frame: InterruptStackFrame, error_code: u64) {
    use crate::arch::x86_64::structures::paging::PageFaultErrorCode;

    let cr2: u64;
    // SAFETY: Reading CR2 is always safe during a page fault handler.
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
    }

    let error = PageFaultErrorCode::from_bits_truncate(error_code);

    let access = if error.contains(PageFaultErrorCode::INSTRUCTION_FETCH) {
        "instruction fetch"
    } else if error.contains(PageFaultErrorCode::WRITE) {
        "write"
    } else {
        "read"
    };

    let cause = if error.contains(PageFaultErrorCode::PRESENT) {
        "protection violation"
    } else {
        "page not present"
    };

    let is_user = error.contains(PageFaultErrorCode::USER);
    let mode = if is_user { "user" } else { "kernel" };

    panic!(
        "PAGE FAULT: {cause} during {mode} {access}\n  \
         Address: {cr2:#x}\n  Error: {error:?}\n{frame:#?}"
    );
}

pub extern "x86-interrupt" fn x87_floating_point(frame: InterruptStackFrame) {
    panic!("EXCEPTION: x87 FLOATING-POINT\n{:#?}", frame);
}

pub extern "x86-interrupt" fn alignment_check(frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: ALIGNMENT CHECK (error_code={:#x})\n{:#?}",
        error_code, frame
    );
}

pub extern "x86-interrupt" fn machine_check(frame: InterruptStackFrame) -> ! {
    panic!("EXCEPTION: MACHINE CHECK\n{:#?}", frame);
}

pub extern "x86-interrupt" fn simd_floating_point(frame: InterruptStackFrame) {
    panic!("EXCEPTION: SIMD FLOATING-POINT\n{:#?}", frame);
}

pub extern "x86-interrupt" fn virtualization(frame: InterruptStackFrame) {
    panic!("EXCEPTION: VIRTUALIZATION\n{:#?}", frame);
}

pub extern "x86-interrupt" fn control_protection(frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: CONTROL PROTECTION (error_code={:#x})\n{:#?}",
        error_code, frame
    );
}

pub extern "x86-interrupt" fn hypervisor_injection(frame: InterruptStackFrame) {
    panic!("EXCEPTION: HYPERVISOR INJECTION\n{:#?}", frame);
}

pub extern "x86-interrupt" fn vmm_communication(frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: VMM COMMUNICATION (error_code={:#x})\n{:#?}",
        error_code, frame
    );
}

pub extern "x86-interrupt" fn security_exception(frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: SECURITY EXCEPTION (error_code={:#x})\n{:#?}",
        error_code, frame
    );
}
