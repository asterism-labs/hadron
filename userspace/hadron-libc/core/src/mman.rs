//! Memory mapping functions.
//!
//! POSIX functions: `mmap`, `munmap`.

use crate::errno;
use crate::flags;
use crate::sys;

/// Map files or devices into memory.
///
/// # Safety
///
/// Caller must ensure valid `addr`, `len`, `prot`, `flags`, `fd`, `offset`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mmap(
    addr: *mut u8,
    len: usize,
    prot: i32,
    map_flags: i32,
    fd: i32,
    _offset: i64,
) -> *mut u8 {
    if len == 0 {
        errno::set_errno(crate::errno::EINVAL);
        return usize::MAX as *mut u8; // MAP_FAILED
    }
    let hadron_prot = flags::posix_prot_to_hadron(prot as u32);
    let hadron_flags = flags::posix_mmap_to_hadron(map_flags as u32);
    match sys::sys_mmap(addr as usize, len, hadron_prot, hadron_flags, fd as usize) {
        Ok(ptr) if !ptr.is_null() => ptr,
        Ok(_) => {
            errno::set_errno(crate::errno::ENOMEM);
            usize::MAX as *mut u8
        }
        Err(e) => {
            errno::set_errno(e);
            usize::MAX as *mut u8 // MAP_FAILED
        }
    }
}

/// Unmap a previously mapped region.
///
/// # Safety
///
/// `addr` must be a value returned by `mmap`. `len` must match.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn munmap(addr: *mut u8, len: usize) -> i32 {
    match sys::sys_munmap(addr, len) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}
