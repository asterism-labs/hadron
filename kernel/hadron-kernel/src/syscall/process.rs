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

/// Maximum number of arguments that can be passed to a spawned process.
const MAX_SPAWN_ARGS: usize = 32;

/// Maximum total bytes for all argument strings combined.
const MAX_ARGV_TOTAL_BYTES: usize = 4096;

/// `sys_task_spawn` — creates a new process from an ELF binary at the given path.
///
/// If `argv_ptr` and `argv_count` are both 0, no arguments are passed.
/// Otherwise reads `SpawnArg` descriptors from the parent's address space and
/// passes the argument strings to the child process.
///
/// Returns the child PID on success, or a negated errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "PIDs are small u32 values, wrap is impossible"
)]
pub(super) fn sys_task_spawn(
    path_ptr: usize,
    path_len: usize,
    argv_ptr: usize,
    argv_count: usize,
) -> isize {
    let uslice = match UserSlice::new(path_ptr, path_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: The user slice was validated above. We're in a syscall context
    // with the user address space still mapped. The path bytes are read-only.
    let path_bytes = unsafe { uslice.as_slice() };
    let path = match core::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => return -(hadron_core::syscall::EINVAL),
    };

    // Read argv from parent address space.
    let mut arg_storage = [0u8; MAX_ARGV_TOTAL_BYTES];
    let mut arg_offsets = [(0usize, 0usize); MAX_SPAWN_ARGS]; // (offset, len)
    let mut arg_count = 0usize;
    let mut total_bytes = 0usize;

    if argv_ptr != 0 && argv_count != 0 {
        if argv_count > MAX_SPAWN_ARGS {
            return -(hadron_core::syscall::EINVAL);
        }

        let desc_size = core::mem::size_of::<hadron_syscall::SpawnArg>() * argv_count;
        let desc_slice = match UserSlice::new(argv_ptr, desc_size) {
            Ok(s) => s,
            Err(e) => return e,
        };

        // SAFETY: The user slice was validated; user CR3 is still active.
        let desc_bytes = unsafe { desc_slice.as_slice() };
        // SAFETY: SpawnArg is repr(C) with only usize fields; any bit pattern is valid.
        let descs = unsafe {
            core::slice::from_raw_parts(
                desc_bytes.as_ptr().cast::<hadron_syscall::SpawnArg>(),
                argv_count,
            )
        };

        for desc in descs {
            if desc.len == 0 {
                arg_offsets[arg_count] = (total_bytes, 0);
                arg_count += 1;
                continue;
            }
            if total_bytes + desc.len > MAX_ARGV_TOTAL_BYTES {
                return -(hadron_core::syscall::EINVAL);
            }
            let arg_slice = match UserSlice::new(desc.ptr, desc.len) {
                Ok(s) => s,
                Err(e) => return e,
            };
            // SAFETY: Validated by UserSlice; user CR3 still active.
            let arg_bytes = unsafe { arg_slice.as_slice() };
            // Validate UTF-8.
            if core::str::from_utf8(arg_bytes).is_err() {
                return -(hadron_core::syscall::EINVAL);
            }
            arg_storage[total_bytes..total_bytes + desc.len].copy_from_slice(arg_bytes);
            arg_offsets[arg_count] = (total_bytes, desc.len);
            total_bytes += desc.len;
            arg_count += 1;
        }
    }

    // Build &[&str] from the copied argument data.
    // This is safe because we validated UTF-8 above.
    let mut arg_strs: [&str; MAX_SPAWN_ARGS] = [""; MAX_SPAWN_ARGS];
    for i in 0..arg_count {
        let (offset, len) = arg_offsets[i];
        // SAFETY: We validated UTF-8 when copying from userspace.
        arg_strs[i] = unsafe { core::str::from_utf8_unchecked(&arg_storage[offset..offset + len]) };
    }
    let args = &arg_strs[..arg_count];

    let parent_pid = crate::proc::with_current_process(|p| p.pid);

    match crate::proc::exec::spawn_process(path, parent_pid, args) {
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
    #[expect(clippy::cast_possible_truncation, reason = "PID fits in u32")]
    crate::proc::set_wait_params(pid as u32, status_ptr as u64);
    crate::proc::set_trap_reason(crate::proc::TRAP_WAIT);

    let saved_rsp = crate::proc::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save,
    // still valid on the executor stack.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}
