//! Backend for the libc allocator: requests pages from the kernel via `sys_mmap`.

use crate::sys;

/// Request `size` bytes of anonymous read/write memory from the kernel.
///
/// Returns `Some(ptr)` on success or `None` on failure.
pub fn request_pages(size: usize) -> Option<*mut u8> {
    // PROT_READ | PROT_WRITE
    let prot = hadron_syscall::PROT_READ | hadron_syscall::PROT_WRITE;
    // MAP_ANONYMOUS
    let flags = hadron_syscall::MAP_ANONYMOUS;

    match sys::sys_mmap(0, size, prot, flags, 0) {
        Ok(ptr) if !ptr.is_null() => Some(ptr),
        _ => None,
    }
}
