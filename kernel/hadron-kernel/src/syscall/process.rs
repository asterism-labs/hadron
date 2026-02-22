//! Task syscall handlers: task_exit, task_info, task_spawn, task_wait, task_kill.

use crate::arch::x86_64::registers::control::Cr3;
use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
use crate::arch::x86_64::userspace::restore_kernel_context;
use crate::syscall::userptr::UserSlice;

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
    crate::proc::set_trap_reason(crate::proc::TrapReason::Exit);
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
    crate::proc::try_current_process(|process| process.pid.as_u32() as isize).unwrap_or(0)
}

/// Maximum number of arguments that can be passed to a spawned process.
const MAX_SPAWN_ARGS: usize = 32;

/// Maximum total bytes for all argument strings combined.
const MAX_ARGV_TOTAL_BYTES: usize = 4096;

/// Maximum number of environment variables that can be passed.
const MAX_SPAWN_ENVS: usize = 64;

/// Maximum total bytes for all environment strings combined.
const MAX_ENVP_TOTAL_BYTES: usize = 8192;

/// Read an array of `SpawnArg` descriptors from user memory and copy their
/// string data into a kernel buffer. Returns the number of entries copied.
///
/// Returns a negative errno on validation failure.
fn read_spawn_args(
    descs_ptr: usize,
    descs_count: usize,
    max_entries: usize,
    storage: &mut [u8],
    offsets: &mut [(usize, usize)],
) -> Result<(usize, usize), isize> {
    if descs_ptr == 0 || descs_count == 0 {
        return Ok((0, 0));
    }
    if descs_count > max_entries {
        return Err(-(crate::syscall::EINVAL));
    }

    let desc_size = core::mem::size_of::<hadron_syscall::SpawnArg>() * descs_count;
    let desc_slice = UserSlice::new(descs_ptr, desc_size).map_err(|e| e)?;

    // SAFETY: The user slice was validated; user CR3 is still active.
    let desc_bytes = unsafe { desc_slice.as_slice() };
    // SAFETY: SpawnArg is repr(C) with only usize fields; any bit pattern is valid.
    let descs = unsafe {
        core::slice::from_raw_parts(
            desc_bytes.as_ptr().cast::<hadron_syscall::SpawnArg>(),
            descs_count,
        )
    };

    let mut count = 0usize;
    let mut total_bytes = 0usize;

    for desc in descs {
        if desc.len == 0 {
            offsets[count] = (total_bytes, 0);
            count += 1;
            continue;
        }
        if total_bytes + desc.len > storage.len() {
            return Err(-(crate::syscall::EINVAL));
        }
        let arg_slice = UserSlice::new(desc.ptr, desc.len).map_err(|e| e)?;
        // SAFETY: Validated by UserSlice; user CR3 still active.
        let arg_bytes = unsafe { arg_slice.as_slice() };
        if core::str::from_utf8(arg_bytes).is_err() {
            return Err(-(crate::syscall::EINVAL));
        }
        storage[total_bytes..total_bytes + desc.len].copy_from_slice(arg_bytes);
        offsets[count] = (total_bytes, desc.len);
        total_bytes += desc.len;
        count += 1;
    }

    Ok((count, total_bytes))
}

/// Build a `&[&str]` from copied argument data.
fn build_str_slice<'a>(
    storage: &'a [u8],
    offsets: &[(usize, usize)],
    count: usize,
    out: &mut [&'a str],
) {
    for i in 0..count {
        let (offset, len) = offsets[i];
        // SAFETY: We validated UTF-8 when copying from userspace.
        out[i] = unsafe { core::str::from_utf8_unchecked(&storage[offset..offset + len]) };
    }
}

/// `sys_task_spawn` — creates a new process from an ELF binary.
///
/// Reads a [`SpawnInfo`] from user memory, extracts path/argv/envp, and
/// spawns a child process. Returns the child PID on success.
#[expect(
    clippy::cast_possible_wrap,
    reason = "PIDs are small u32 values, wrap is impossible"
)]
pub(super) fn sys_task_spawn(info_ptr: usize, info_len: usize) -> isize {
    let expected_size = core::mem::size_of::<hadron_syscall::SpawnInfo>();
    if info_len < expected_size {
        return -(crate::syscall::EINVAL);
    }

    let info_slice = match UserSlice::new(info_ptr, expected_size) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: UserSlice validated the pointer; SpawnInfo is repr(C) with only
    // usize fields so any bit pattern is valid.
    let info: hadron_syscall::SpawnInfo =
        unsafe { core::ptr::read_unaligned(info_slice.addr() as *const hadron_syscall::SpawnInfo) };

    // Read path.
    let path_uslice = match UserSlice::new(info.path_ptr, info.path_len) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let path_bytes = unsafe { path_uslice.as_slice() };
    let path = match core::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => return -(crate::syscall::EINVAL),
    };

    // Read argv.
    let mut arg_storage = [0u8; MAX_ARGV_TOTAL_BYTES];
    let mut arg_offsets = [(0usize, 0usize); MAX_SPAWN_ARGS];
    let (arg_count, _) = match read_spawn_args(
        info.argv_ptr,
        info.argv_count,
        MAX_SPAWN_ARGS,
        &mut arg_storage,
        &mut arg_offsets,
    ) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let mut arg_strs: [&str; MAX_SPAWN_ARGS] = [""; MAX_SPAWN_ARGS];
    build_str_slice(&arg_storage, &arg_offsets, arg_count, &mut arg_strs);
    let args = &arg_strs[..arg_count];

    // Read envp.
    let mut env_storage = [0u8; MAX_ENVP_TOTAL_BYTES];
    let mut env_offsets = [(0usize, 0usize); MAX_SPAWN_ENVS];
    let (env_count, _) = match read_spawn_args(
        info.envp_ptr,
        info.envp_count,
        MAX_SPAWN_ENVS,
        &mut env_storage,
        &mut env_offsets,
    ) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let mut env_strs: [&str; MAX_SPAWN_ENVS] = [""; MAX_SPAWN_ENVS];
    build_str_slice(&env_storage, &env_offsets, env_count, &mut env_strs);
    let envs = &env_strs[..env_count];

    let parent_pid = crate::proc::with_current_process(|p| p.pid);

    match crate::proc::exec::spawn_process(path, parent_pid, args, envs) {
        Ok(child) => child.pid.as_u32() as isize,
        Err(_) => -(crate::syscall::ENOENT),
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
    crate::proc::set_wait_params(crate::id::Pid::new(pid as u32), status_ptr as u64);
    crate::proc::set_trap_reason(crate::proc::TrapReason::Wait);

    let saved_rsp = crate::proc::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save,
    // still valid on the executor stack.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}

/// `sys_task_kill` — sends a signal to a process.
///
/// Returns 0 on success, or a negated errno on failure.
#[expect(clippy::cast_possible_truncation, reason = "PID fits in u32")]
pub(super) fn sys_task_kill(pid: usize, signum: usize) -> isize {
    use crate::proc::signal::Signal;

    if !Signal::is_valid(signum) {
        return -(crate::syscall::EINVAL);
    }

    let target = crate::proc::lookup_process(crate::id::Pid::new(pid as u32));
    match target {
        Some(proc) => {
            proc.signals.post(signum);
            0
        }
        None => -(crate::syscall::EINVAL),
    }
}
