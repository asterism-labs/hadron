//! Syscall dispatch table.
//!
//! Routes incoming syscall numbers to individual handler functions.
//! Uses the native Hadron syscall ABI with grouped numbering.

mod io;
mod memory;
mod process;
mod time;
mod vfs;

use hadron_core::syscall::*;

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
    _a3: usize,
    _a4: usize,
) -> isize {
    match nr {
        // Task management
        SYS_TASK_EXIT => process::sys_task_exit(a0),
        SYS_TASK_INFO => process::sys_task_info(),

        // Filesystem
        SYS_VNODE_OPEN => vfs::sys_vnode_open(a0, a1, a2),
        SYS_VNODE_READ => vfs::sys_vnode_read(a0, a1, a2),
        SYS_VNODE_WRITE => vfs::sys_vnode_write(a0, a1, a2),

        // Memory
        SYS_MEM_MAP => memory::sys_mem_map(),
        SYS_MEM_UNMAP => memory::sys_mem_unmap(),

        // Time
        SYS_CLOCK_GETTIME => time::sys_clock_gettime(a0, a1),

        // System / debug
        SYS_DEBUG_LOG => io::sys_debug_log(a0, a1),

        _ => -ENOSYS,
    }
}
