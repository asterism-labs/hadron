//! Time syscall handlers: clock_gettime.

use hadron_core::syscall::userptr::{UserPtr, is_kernel_caller};
use hadron_core::syscall::{Timespec, CLOCK_MONOTONIC, EFAULT, EINVAL};

/// `sys_clock_gettime` — returns boot-relative monotonic time.
///
/// Reads the HPET-backed `boot_nanos()` time source and writes a [`Timespec`]
/// to the caller-supplied buffer. Only `CLOCK_MONOTONIC` (0) is supported.
pub(super) fn sys_clock_gettime(clock_id: usize, tp: usize) -> isize {
    if clock_id != CLOCK_MONOTONIC {
        return -EINVAL;
    }

    let nanos = crate::time::boot_nanos();
    let ts = Timespec {
        tv_sec: nanos / 1_000_000_000,
        tv_nsec: nanos % 1_000_000_000,
    };

    if is_kernel_caller(tp) {
        // Kernel-mode test: tp is a kernel address, write directly.
        // SAFETY: Kernel-mode callers pass a valid stack-local `&mut Timespec`.
        // The address is in the upper half (verified by `is_kernel_caller`).
        unsafe { core::ptr::write(tp as *mut Timespec, ts) };
    } else {
        let user_ptr = match UserPtr::<Timespec>::new(tp) {
            Ok(p) => p,
            Err(_) => return -EFAULT,
        };
        // SAFETY: UserPtr validated that the address is in user space,
        // properly aligned, and does not overflow. The caller is responsible
        // for ensuring the page is mapped and writable.
        unsafe { core::ptr::write(user_ptr.addr() as *mut Timespec, ts) };
    }

    0
}
