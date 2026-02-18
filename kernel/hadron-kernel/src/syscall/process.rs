//! Task syscall handlers: task_exit, task_info.

use hadron_core::arch::x86_64::registers::control::Cr3;
use hadron_core::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
use hadron_core::arch::x86_64::userspace::restore_kernel_context;

/// `sys_task_exit` — terminates the current user process.
///
/// Restores the kernel address space and GS bases, stores the exit status,
/// then calls `restore_kernel_context` to "return" from `enter_userspace_save`
/// back into the process task on the executor.
pub(super) fn sys_task_exit(status: usize) -> isize {
    let kernel_cr3 = crate::proc::kernel_cr3();

    unsafe {
        // Restore kernel address space.
        Cr3::write(kernel_cr3);

        // Restore GS bases to both-point-to-percpu state.
        let percpu = IA32_GS_BASE.read(); // currently kernel GS (set by swapgs in syscall_entry)
        IA32_KERNEL_GS_BASE.write(percpu);
    }

    // Store exit status and trap reason, then jump back to the process task.
    crate::proc::set_process_exit_status(status as u64);
    crate::proc::set_trap_reason(crate::proc::TRAP_EXIT);
    let saved_rsp = crate::proc::saved_kernel_rsp();

    unsafe {
        restore_kernel_context(saved_rsp);
    }
}

/// `sys_task_info` — returns the current process ID (PID).
#[expect(
    clippy::cast_possible_wrap,
    reason = "PIDs are small u32 values, wrap is impossible"
)]
pub(super) fn sys_task_info() -> isize {
    crate::proc::with_current_process(|process| process.pid as isize)
}
