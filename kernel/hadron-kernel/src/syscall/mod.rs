//! Syscall dispatch table and userspace pointer validation.
//!
//! Routes incoming syscall numbers to individual handler functions via the
//! generated [`SyscallHandler`] trait from `hadron-syscall`.

mod event;
mod io;
mod ioctl;
mod memory;
mod process;
mod query;
mod time;
pub mod userptr;
mod vfs;

pub use hadron_syscall::*;

/// Kernel syscall handler implementation.
///
/// Delegates each syscall to the corresponding handler module.
struct HadronDispatch;

impl SyscallHandler for HadronDispatch {
    fn sys_task_exit(&self, status: usize) -> isize {
        process::sys_task_exit(status)
    }

    fn sys_task_spawn(&self, info_ptr: usize, info_len: usize) -> isize {
        process::sys_task_spawn(info_ptr, info_len)
    }

    fn sys_task_wait(&self, pid: usize, status_ptr: usize, flags: usize) -> isize {
        process::sys_task_wait(pid, status_ptr, flags)
    }

    fn sys_task_kill(&self, pid: usize, signum: usize) -> isize {
        process::sys_task_kill(pid, signum)
    }

    fn sys_task_info(&self) -> isize {
        process::sys_task_info()
    }

    fn sys_task_sigaction(
        &self,
        signum: usize,
        handler: usize,
        flags: usize,
        old_handler_out: usize,
    ) -> isize {
        process::sys_task_sigaction(signum, handler, flags, old_handler_out)
    }

    fn sys_task_sigreturn(&self) -> isize {
        process::sys_task_sigreturn()
    }

    fn sys_task_setpgid(&self, pid: usize, pgid: usize) -> isize {
        process::sys_task_setpgid(pid, pgid)
    }

    fn sys_task_getpgid(&self, pid: usize) -> isize {
        process::sys_task_getpgid(pid)
    }

    fn sys_task_getppid(&self) -> isize {
        process::sys_task_getppid()
    }

    fn sys_task_getcwd(&self, buf_ptr: usize, buf_len: usize) -> isize {
        process::sys_task_getcwd(buf_ptr, buf_len)
    }

    fn sys_task_chdir(&self, path_ptr: usize, path_len: usize) -> isize {
        process::sys_task_chdir(path_ptr, path_len)
    }

    fn sys_task_setsid(&self) -> isize {
        process::sys_task_setsid()
    }

    fn sys_task_sigprocmask(&self, how: usize, set: usize, oldset_out: usize) -> isize {
        process::sys_task_sigprocmask(how, set, oldset_out)
    }

    fn sys_task_execve(&self, info_ptr: usize, info_len: usize) -> isize {
        process::sys_task_execve(info_ptr, info_len)
    }

    fn sys_handle_close(&self, handle: usize) -> isize {
        vfs::sys_handle_close(handle)
    }

    fn sys_handle_dup(&self, old_fd: usize, new_fd: usize) -> isize {
        vfs::sys_handle_dup(old_fd, new_fd)
    }

    fn sys_handle_dup_lowest(&self, old_fd: usize) -> isize {
        vfs::sys_handle_dup_lowest(old_fd)
    }

    fn sys_handle_pipe(&self, fds_ptr: usize) -> isize {
        vfs::sys_handle_pipe(fds_ptr)
    }

    fn sys_handle_tcsetpgrp(&self, fd: usize, pgid: usize) -> isize {
        vfs::sys_handle_tcsetpgrp(fd, pgid)
    }

    fn sys_handle_tcgetpgrp(&self, fd: usize) -> isize {
        vfs::sys_handle_tcgetpgrp(fd)
    }

    fn sys_handle_ioctl(&self, fd: usize, cmd: usize, arg_ptr: usize) -> isize {
        ioctl::sys_handle_ioctl(fd, cmd, arg_ptr)
    }

    fn sys_handle_fcntl(&self, fd: usize, cmd: usize, arg: usize) -> isize {
        vfs::sys_handle_fcntl(fd, cmd, arg)
    }

    fn sys_handle_pipe2(&self, fds_ptr: usize, flags: usize) -> isize {
        vfs::sys_handle_pipe2(fds_ptr, flags)
    }

    fn sys_vnode_open(&self, path_ptr: usize, path_len: usize, flags: usize) -> isize {
        vfs::sys_vnode_open(path_ptr, path_len, flags)
    }

    fn sys_vnode_read(&self, fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
        vfs::sys_vnode_read(fd, buf_ptr, buf_len)
    }

    fn sys_vnode_write(&self, fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
        vfs::sys_vnode_write(fd, buf_ptr, buf_len)
    }

    fn sys_vnode_stat(&self, fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
        vfs::sys_vnode_stat(fd, buf_ptr, buf_len)
    }

    fn sys_vnode_readdir(&self, fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
        vfs::sys_vnode_readdir(fd, buf_ptr, buf_len)
    }

    fn sys_vnode_unlink(&self, path_ptr: usize, path_len: usize) -> isize {
        vfs::sys_vnode_unlink(path_ptr, path_len)
    }

    fn sys_vnode_seek(&self, fd: usize, offset: usize, whence: usize) -> isize {
        vfs::sys_vnode_seek(fd, offset, whence)
    }

    fn sys_vnode_mkdir(&self, path_ptr: usize, path_len: usize, permissions: usize) -> isize {
        vfs::sys_vnode_mkdir(path_ptr, path_len, permissions)
    }

    fn sys_vnode_rename(
        &self,
        old_ptr: usize,
        old_len: usize,
        new_ptr: usize,
        new_len: usize,
    ) -> isize {
        vfs::sys_vnode_rename(old_ptr, old_len, new_ptr, new_len)
    }

    fn sys_vnode_symlink(
        &self,
        target_ptr: usize,
        target_len: usize,
        link_ptr: usize,
        link_len: usize,
    ) -> isize {
        vfs::sys_vnode_symlink(target_ptr, target_len, link_ptr, link_len)
    }

    fn sys_vnode_link(
        &self,
        target_ptr: usize,
        target_len: usize,
        link_ptr: usize,
        link_len: usize,
    ) -> isize {
        vfs::sys_vnode_link(target_ptr, target_len, link_ptr, link_len)
    }

    fn sys_vnode_readlink(
        &self,
        path_ptr: usize,
        path_len: usize,
        buf_ptr: usize,
        buf_len: usize,
    ) -> isize {
        vfs::sys_vnode_readlink(path_ptr, path_len, buf_ptr, buf_len)
    }

    fn sys_vnode_truncate(&self, fd: usize, len: usize) -> isize {
        vfs::sys_vnode_truncate(fd, len)
    }

    fn sys_vnode_fstatat(
        &self,
        dirfd: usize,
        path_ptr: usize,
        path_len: usize,
        buf: usize,
        flags: usize,
    ) -> isize {
        vfs::sys_vnode_fstatat(dirfd, path_ptr, path_len, buf, flags)
    }

    fn sys_mem_map(
        &self,
        addr_hint: usize,
        length: usize,
        prot: usize,
        flags: usize,
        fd: usize,
    ) -> isize {
        memory::sys_mem_map(addr_hint, length, prot, flags, fd)
    }

    fn sys_mem_unmap(&self, addr: usize, length: usize) -> isize {
        memory::sys_mem_unmap(addr, length)
    }

    fn sys_mem_brk(&self, addr: usize) -> isize {
        memory::sys_mem_brk(addr)
    }

    fn sys_clock_gettime(&self, clock_id: usize, tp: usize) -> isize {
        time::sys_clock_gettime(clock_id, tp)
    }

    fn sys_clock_nanosleep(
        &self,
        clock_id: usize,
        flags: usize,
        req_ptr: usize,
        rem_ptr: usize,
    ) -> isize {
        time::sys_clock_nanosleep(clock_id, flags, req_ptr, rem_ptr)
    }

    fn sys_task_clone(&self, flags: usize, stack_ptr: usize, tls_ptr: usize) -> isize {
        process::sys_task_clone(flags, stack_ptr, tls_ptr)
    }

    fn sys_event_wait_many(&self, fds_ptr: usize, nfds: usize, timeout_ms: usize) -> isize {
        event::sys_event_wait_many(fds_ptr, nfds, timeout_ms)
    }

    fn sys_futex(&self, addr: usize, op: usize, val: usize, timeout_ms: usize) -> isize {
        event::sys_futex(addr, op, val, timeout_ms)
    }

    fn sys_query(&self, topic: usize, sub_id: usize, out_buf: usize, out_len: usize) -> isize {
        query::sys_query(topic, sub_id, out_buf, out_len)
    }

    fn sys_debug_log(&self, buf: usize, len: usize) -> isize {
        io::sys_debug_log(buf, len)
    }
}

/// Global dispatch instance.
static DISPATCH: HadronDispatch = HadronDispatch;

/// Syscall dispatch entry point, called from the assembly stub in `hadron-core`.
///
/// Matches the syscall number and forwards to the appropriate handler.
/// Unknown syscall numbers return `-ENOSYS`.
///
/// After the handler returns, processes any pending keyboard input and
/// checks for pending signals. This ensures Ctrl+C is recognised even
/// during tight syscall loops where the normal TTY read path never runs.
#[unsafe(no_mangle)]
extern "C" fn syscall_dispatch(
    nr: usize,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
) -> isize {
    crate::ktrace_subsys!(syscall, "syscall nr={} a0={:#x} a1={:#x}", nr, a0, a1);
    let result = dispatch(&DISPATCH, nr, a0, a1, a2, a3, a4);

    // Process any keyboard input that arrived during this syscall
    // (the keyboard IRQ can now fire thanks to STI in the entry stub).
    crate::tty::process_pending_input();

    // If the current process has pending signals, longjmp back to
    // process_task for delivery instead of returning via sysretq.
    let has_signal = crate::proc::ProcessTable::try_current(|p| p.signals.has_pending());
    if has_signal == Some(true) {
        trap_signal_pending(result);
    }

    result
}

/// Longjmp back to `process_task` for signal delivery.
///
/// Populates `USER_CONTEXT` from `SYSCALL_SAVED_REGS` + the syscall
/// return value, restores kernel CR3 and GS, sets `TrapReason::Preempted`,
/// and calls `restore_kernel_context`. process_task will then check
/// signals and either terminate or deliver a handler.
fn trap_signal_pending(result: isize) -> ! {
    use crate::arch::x86_64::registers::control::Cr3;
    use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
    use crate::arch::x86_64::userspace::restore_kernel_context;

    // Populate USER_CONTEXT from saved syscall registers + result.
    // SAFETY: USER_CONTEXT and SYSCALL_SAVED_REGS are per-CPU, only
    // accessed from this task and the preemption stub (mutually exclusive).
    unsafe {
        let saved = &*crate::arch::x86_64::syscall::SYSCALL_SAVED_REGS.get().get();
        let ctx = &mut *crate::proc::USER_CONTEXT.get().get();
        ctx.rip = saved.user_rip;
        ctx.rflags = saved.user_rflags;
        ctx.rsp = crate::percpu::PerCpuState::current().user_rsp;
        ctx.rbx = saved.rbx;
        ctx.rbp = saved.rbp;
        ctx.r12 = saved.r12;
        ctx.r13 = saved.r13;
        ctx.r14 = saved.r14;
        ctx.r15 = saved.r15;
        ctx.rax = result as u64;
        // Caller-saved registers are clobbered by the syscall ABI.
        ctx.rcx = 0;
        ctx.rdx = 0;
        ctx.rsi = 0;
        ctx.rdi = 0;
        ctx.r8 = 0;
        ctx.r9 = 0;
        ctx.r10 = 0;
        ctx.r11 = 0;
    }

    let kernel_cr3 = crate::proc::TrapContext::kernel_cr3();

    // SAFETY: Standard kernel context restore pattern (same as trap_io / sys_task_sigreturn).
    unsafe {
        Cr3::write(kernel_cr3);
        let percpu = IA32_GS_BASE.read();
        IA32_KERNEL_GS_BASE.write(percpu);
    }

    crate::proc::TrapContext::set_trap_reason(crate::proc::TrapReason::Preempted);

    let saved_rsp = crate::proc::TrapContext::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}
