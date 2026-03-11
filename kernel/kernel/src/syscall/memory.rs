//! Memory syscall handlers: map, unmap, brk.
//!
//! Phase 2b stubs — full implementation deferred to Phase 2c.

use hadron_syscall::*;

/// `SYS_MEM_MAP` — stub (not yet implemented).
pub fn sys_mem_map(
    _addr_hint: usize,
    _len: usize,
    _prot: usize,
    _flags: usize,
    _fd: usize,
) -> isize {
    -ENOSYS
}

/// `SYS_MEM_UNMAP` — stub (not yet implemented).
pub fn sys_mem_unmap(_addr: usize, _len: usize) -> isize {
    -ENOSYS
}

/// `SYS_MEM_BRK` — stub (not yet implemented).
pub fn sys_mem_brk(_addr: usize) -> isize {
    -ENOSYS
}
