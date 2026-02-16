//! I/O syscall handlers: debug_log.

use hadron_core::syscall::userptr::UserSlice;
use hadron_core::syscall::EFAULT;

/// `sys_debug_log` — writes a message to the kernel serial console.
///
/// Takes a pointer (`buf`) and length (`len`) and writes the data via
/// `kprint!`. Validates the buffer via [`UserSlice`] to ensure it lies
/// within user address space. Kernel-mode test callers (detected by
/// checking whether `buf` is in the upper half) bypass validation since
/// they pass kernel addresses.
pub(super) fn sys_debug_log(buf: usize, len: usize) -> isize {
    // SAFETY: The buffer is validated to be in user space (or known to be
    // in kernel space during early kernel-mode testing). The memory is
    // readable because the caller just passed it.
    let user_slice;
    let slice = if hadron_core::syscall::userptr::is_kernel_caller(buf) {
        // Kernel-mode test: buf is a kernel address, skip user-space check.
        // Validate that buf + len doesn't overflow and len is reasonable.
        if len == 0 {
            return 0;
        }
        if buf.checked_add(len).is_none() {
            return -EFAULT;
        }
        // SAFETY: Verified that buf is a kernel address (upper-half), len > 0,
        // and buf + len does not overflow. The caller passed this buffer, so
        // the memory is readable for the given length.
        unsafe { core::slice::from_raw_parts(buf as *const u8, len) }
    } else {
        user_slice = match UserSlice::new(buf, len) {
            Ok(s) => s,
            Err(_) => return -EFAULT,
        };
        // SAFETY: UserSlice validated that [buf, buf+len) is in user space.
        unsafe { user_slice.as_slice() }
    };

    if let Ok(s) = core::str::from_utf8(slice) {
        hadron_core::kprint!("{}", s);
    } else {
        for &byte in slice {
            hadron_core::kprint!("{}", byte as char);
        }
    }
    len as isize
}
