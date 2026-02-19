//! Syscall dispatch table.
//!
//! Routes incoming syscall numbers to individual handler functions via the
//! generated [`SyscallHandler`] trait from `hadron-syscall`.

mod io;
mod memory;
mod process;
mod query;
mod time;
mod vfs;

use hadron_syscall::{SyscallHandler, dispatch};

/// Kernel syscall handler implementation.
///
/// Delegates each syscall to the corresponding handler module.
struct HadronDispatch;

impl SyscallHandler for HadronDispatch {
    fn sys_task_exit(&self, status: usize) -> isize {
        process::sys_task_exit(status)
    }

    fn sys_task_spawn(
        &self,
        path_ptr: usize,
        path_len: usize,
        argv_ptr: usize,
        argv_count: usize,
    ) -> isize {
        process::sys_task_spawn(path_ptr, path_len, argv_ptr, argv_count)
    }

    fn sys_task_wait(&self, pid: usize, status_ptr: usize) -> isize {
        process::sys_task_wait(pid, status_ptr)
    }

    fn sys_task_info(&self) -> isize {
        process::sys_task_info()
    }

    fn sys_handle_close(&self, handle: usize) -> isize {
        vfs::sys_handle_close(handle)
    }

    fn sys_handle_dup(&self, old_fd: usize, new_fd: usize) -> isize {
        vfs::sys_handle_dup(old_fd, new_fd)
    }

    fn sys_handle_pipe(&self, fds_ptr: usize) -> isize {
        vfs::sys_handle_pipe(fds_ptr)
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

    fn sys_mem_map(&self) -> isize {
        memory::sys_mem_map()
    }

    fn sys_mem_unmap(&self) -> isize {
        memory::sys_mem_unmap()
    }

    fn sys_clock_gettime(&self, clock_id: usize, tp: usize) -> isize {
        time::sys_clock_gettime(clock_id, tp)
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
#[unsafe(no_mangle)]
extern "C" fn syscall_dispatch(
    nr: usize,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
) -> isize {
    dispatch(&DISPATCH, nr, a0, a1, a2, a3, a4)
}
