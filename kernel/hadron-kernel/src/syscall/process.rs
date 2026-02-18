//! Task syscall handlers: task_exit, task_info, task_spawn, task_wait.

use hadron_core::arch::x86_64::registers::control::Cr3;
use hadron_core::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
use hadron_core::arch::x86_64::userspace::restore_kernel_context;
use hadron_core::syscall::userptr::UserSlice;

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
///
/// Returns 0 if no process is running (kernel context / test harness).
#[expect(
    clippy::cast_possible_wrap,
    reason = "PIDs are small u32 values, wrap is impossible"
)]
pub(super) fn sys_task_info() -> isize {
    crate::proc::try_current_process(|process| process.pid as isize).unwrap_or(0)
}

/// `sys_task_spawn` — creates a new process from an ELF binary at the given path.
///
/// Returns the child PID on success, or a negated errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "PIDs are small u32 values, wrap is impossible"
)]
pub(super) fn sys_task_spawn(path_ptr: usize, path_len: usize) -> isize {
    let uslice = match UserSlice::new(path_ptr, path_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: The user slice was validated above. We're in a syscall context
    // with the user address space still mapped (GS was swapped but CR3 is
    // user CR3 during syscall — actually no, after the syscall entry stub we
    // are still on the user's page tables for the data segment). The path
    // bytes are read-only.
    let path_bytes = unsafe { uslice.as_slice() };
    let path = match core::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => return -(hadron_core::syscall::EINVAL),
    };

    let parent_pid = crate::proc::with_current_process(|p| p.pid);

    match crate::proc::exec::spawn_process(path, parent_pid) {
        Ok(child) => child.pid as isize,
        Err(_) => -(hadron_core::syscall::ENOENT),
    }
}

/// `sys_task_wait` — waits for a child process to exit.
///
/// This is a blocking syscall implemented via the TRAP_WAIT mechanism.
/// Sets up the wait parameters and longjmps back to `process_task`,
/// which handles the async wait in its event loop.
///
/// Never returns to the caller — execution resumes when `process_task`
/// re-enters userspace with the result in RAX.
pub(super) fn sys_task_wait(pid: usize, status_ptr: usize) -> isize {
    // Validate status_ptr if non-null.
    if status_ptr != 0 {
        if let Err(e) = UserSlice::new(status_ptr, core::mem::size_of::<u64>()) {
            return e;
        }
    }

    let kernel_cr3 = crate::proc::kernel_cr3();

    // SAFETY: Restoring kernel CR3 and GS bases is the standard pattern
    // for returning from userspace context to kernel context (same as
    // sys_task_exit).
    unsafe {
        Cr3::write(kernel_cr3);
        let percpu = IA32_GS_BASE.read();
        IA32_KERNEL_GS_BASE.write(percpu);
    }

    // Set up wait parameters for process_task to read.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "PID fits in u32"
    )]
    crate::proc::set_wait_params(pid as u32, status_ptr as u64);
    crate::proc::set_trap_reason(crate::proc::TRAP_WAIT);

    let saved_rsp = crate::proc::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save,
    // still valid on the executor stack.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}
