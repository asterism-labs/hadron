//! Memory syscall handlers: mem_map, mem_unmap.
//!
//! Stubs returning `-ENOSYS` until the process memory model is implemented.

use hadron_core::syscall::ENOSYS;

/// `sys_mem_map` — stub, returns `-ENOSYS`.
pub(super) fn sys_mem_map() -> isize {
    -ENOSYS
}

/// `sys_mem_unmap` — stub, returns `-ENOSYS`.
pub(super) fn sys_mem_unmap() -> isize {
    -ENOSYS
}
