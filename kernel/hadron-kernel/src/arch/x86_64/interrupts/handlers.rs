//! Default CPU exception handlers.
//!
//! All handlers use the `x86-interrupt` ABI. Most panic with the exception name
//! and stack frame. `debug` and `breakpoint` log and return for debugging.

// Handler names are self-documenting; suppress missing_docs for this module.
#![allow(missing_docs)]

use hadron_core::arch::x86_64::structures::idt::InterruptStackFrame;

pub extern "x86-interrupt" fn divide_error(frame: InterruptStackFrame) {
    panic!("EXCEPTION: DIVIDE ERROR\n{:#?}", frame);
}

pub extern "x86-interrupt" fn debug(frame: InterruptStackFrame) {
    hadron_core::kwarn!("EXCEPTION: DEBUG\n{:#?}", frame);
}

pub extern "x86-interrupt" fn nmi(frame: InterruptStackFrame) {
    panic!("EXCEPTION: NON-MASKABLE INTERRUPT\n{:#?}", frame);
}

pub extern "x86-interrupt" fn breakpoint(frame: InterruptStackFrame) {
    hadron_core::kwarn!("EXCEPTION: BREAKPOINT\n{:#?}", frame);
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
    use hadron_core::arch::x86_64::structures::paging::PageFaultErrorCode;
    use hadron_core::mm::layout::FaultRegion;

    let cr2: u64;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
    }

    let error = PageFaultErrorCode::from_bits_truncate(error_code);

    // Corrupted page table — unrecoverable.
    if error.contains(PageFaultErrorCode::RESERVED_WRITE) {
        panic!(
            "PAGE FAULT: corrupted page table (reserved bit set)\n  \
             Address: {cr2:#x}\n  Error: {error:?}\n{frame:#?}"
        );
    }

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

    // User-mode fault: log and terminate the process instead of panicking.
    if is_user {
        hadron_core::kerr!(
            "USER PAGE FAULT: {cause} during {access}\n  \
             Address: {cr2:#x}\n  Error: {error:?}\n{frame:#?}"
        );

        // Restore kernel address space and GS bases.
        let kernel_cr3 = crate::proc::kernel_cr3();
        unsafe {
            hadron_core::arch::x86_64::registers::control::Cr3::write(kernel_cr3);
            let percpu =
                hadron_core::arch::x86_64::registers::model_specific::IA32_KERNEL_GS_BASE.read();
            hadron_core::arch::x86_64::registers::model_specific::IA32_GS_BASE.write(percpu);
        }
        hadron_core::kinfo!("Process terminated due to page fault");

        // Store fault sentinel and restore kernel context to return to process task.
        crate::proc::set_process_exit_status(usize::MAX as u64);
        let saved_rsp = crate::proc::saved_kernel_rsp();
        unsafe {
            hadron_core::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
        }
    }

    // Kernel-mode fault: diagnose and panic.
    // Try to identify the faulting region (non-blocking to avoid deadlock
    // if we faulted inside the VMM itself).
    let region_info = crate::mm::vmm::try_with_vmm(|vmm| {
        let addr = hadron_core::addr::VirtAddr::new_truncate(cr2);
        let layout = vmm.layout();
        let region = layout.identify_region(addr);

        // Guard page hit → stack overflow.
        if region == FaultRegion::Stacks {
            let stacks_base = layout.stacks.base().as_u64();
            let watermark = vmm.stacks_watermark().as_u64();

            // Only check addresses below the watermark (allocated stacks).
            if cr2 >= stacks_base && cr2 < watermark {
                let offset = cr2 - stacks_base;
                // Each stack slot is 4 KiB guard + 64 KiB stack = 68 KiB.
                let slot_size: u64 = 4096 + 64 * 1024;
                let slot_offset = offset % slot_size;

                if slot_offset < 4096 {
                    let stack_index = offset / slot_size;
                    let stack_bottom = stacks_base + stack_index * slot_size + 4096;
                    let stack_top = stack_bottom + 64 * 1024;
                    panic!(
                        "STACK OVERFLOW: guard page hit\n  \
                         Stack index: {stack_index}\n  \
                         Stack bounds: {stack_bottom:#x}..{stack_top:#x}\n  \
                         Faulting address: {cr2:#x}\n  \
                         Error: {error:?}\n{frame:#?}"
                    );
                }
            }
        }

        // Demand-page candidate in heap (foundation for future demand paging).
        if region == FaultRegion::Heap && !error.contains(PageFaultErrorCode::PRESENT) {
            let watermark = vmm.heap_watermark().as_u64();
            if cr2 >= layout.heap.base().as_u64() && cr2 < watermark {
                crate::kwarn!(
                    "page fault: demand-page candidate in heap at {cr2:#x} (not yet implemented)"
                );
            }
        }

        region
    });

    match region_info {
        Some(region) => panic!(
            "PAGE FAULT: {cause} during {mode} {access}\n  \
             Address: {cr2:#x}\n  Region: {region:?}\n  Error: {error:?}\n{frame:#?}"
        ),
        None => panic!(
            "PAGE FAULT: {cause} during {mode} {access} (VMM locked)\n  \
             Address: {cr2:#x}\n  Error: {error:?}\n{frame:#?}"
        ),
    }
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
