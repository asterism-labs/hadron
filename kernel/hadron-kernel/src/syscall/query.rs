//! System query syscall handler.
//!
//! Implements `sys_query`, which returns typed `#[repr(C)]` structs for
//! system information queries, replacing the text-based procfs approach.

use core::mem::size_of;

use crate::mm::PAGE_SIZE;
use crate::syscall::userptr::is_kernel_caller;
use crate::syscall::{
    EFAULT, EINVAL, KernelVersionInfo, MemoryInfo, QUERY_KERNEL_VERSION, QUERY_MEMORY,
    QUERY_UPTIME, UptimeInfo,
};

/// Kernel version: major.
const VERSION_MAJOR: u16 = 0;
/// Kernel version: minor.
const VERSION_MINOR: u16 = 1;
/// Kernel version: patch.
const VERSION_PATCH: u16 = 0;
/// Kernel name bytes, NUL-padded to 32 bytes.
const KERNEL_NAME: &[u8; 6] = b"Hadron";

/// `sys_query(topic, sub_id, out_buf, out_len) -> isize`
///
/// Writes a fixed-layout response struct into `out_buf`.
/// Returns bytes written on success, or `-errno` on failure.
///
/// # Arguments
///
/// * `topic`   — one of the `QUERY_*` constants selecting the information type.
/// * `_sub_id` — reserved for future per-topic sub-selection (e.g. per-CPU info).
/// * `out_buf` — user-space pointer to the output buffer.
/// * `out_len` — size of the output buffer in bytes.
pub(super) fn sys_query(topic: usize, _sub_id: usize, out_buf: usize, out_len: usize) -> isize {
    #[expect(clippy::cast_possible_truncation, reason = "query topics fit in u64")]
    let topic = topic as u64;

    match topic {
        QUERY_MEMORY => query_memory(out_buf, out_len),
        QUERY_UPTIME => query_uptime(out_buf, out_len),
        QUERY_KERNEL_VERSION => query_kernel_version(out_buf, out_len),
        _ => -EINVAL,
    }
}

/// Write a `T` to the caller's output buffer, handling both kernel-mode and
/// user-mode callers.
///
/// Returns `size_of::<T>()` on success or `-errno` on failure.
fn write_response<T>(out_buf: usize, out_len: usize, value: &T) -> isize {
    let needed = size_of::<T>();
    if out_len < needed {
        return -EINVAL;
    }

    if is_kernel_caller(out_buf) {
        // Kernel-mode test: out_buf is a kernel address, write directly.
        // SAFETY: Kernel-mode callers pass a valid pointer in the upper half
        // (verified by `is_kernel_caller`). The caller ensures the buffer is
        // large enough.
        unsafe { core::ptr::write(out_buf as *mut T, core::ptr::read(value)) };
    } else {
        use crate::syscall::userptr::UserPtr;
        let Ok(user_ptr) = UserPtr::<T>::new(out_buf) else {
            return -EFAULT;
        };
        // SAFETY: UserPtr validated that the address is in user space,
        // properly aligned, and does not overflow. The caller is responsible
        // for ensuring the page is mapped and writable.
        unsafe { core::ptr::write(user_ptr.addr() as *mut T, core::ptr::read(value)) };
    }

    // Response structs are small fixed-size types; their size always fits in isize.
    #[expect(
        clippy::cast_possible_wrap,
        reason = "query result size is small, wrap is impossible"
    )]
    let written = needed as isize;
    written
}

/// Handle `QUERY_MEMORY`: return physical memory statistics.
fn query_memory(out_buf: usize, out_len: usize) -> isize {
    let (total_frames, free_frames) =
        crate::mm::pmm::with_pmm(|pmm| (pmm.total_frames(), pmm.free_frames()));

    let info = MemoryInfo {
        total_bytes: (total_frames * PAGE_SIZE) as u64,
        free_bytes: (free_frames * PAGE_SIZE) as u64,
        used_bytes: ((total_frames - free_frames) * PAGE_SIZE) as u64,
    };

    write_response(out_buf, out_len, &info)
}

/// Handle `QUERY_UPTIME`: return nanoseconds since boot.
fn query_uptime(out_buf: usize, out_len: usize) -> isize {
    let info = UptimeInfo {
        uptime_ns: crate::time::boot_nanos(),
    };

    write_response(out_buf, out_len, &info)
}

/// Handle `QUERY_KERNEL_VERSION`: return kernel version metadata.
fn query_kernel_version(out_buf: usize, out_len: usize) -> isize {
    let mut name = [0u8; 32];
    let len = KERNEL_NAME.len().min(name.len());
    name[..len].copy_from_slice(&KERNEL_NAME[..len]);

    let info = KernelVersionInfo {
        major: VERSION_MAJOR,
        minor: VERSION_MINOR,
        patch: VERSION_PATCH,
        _pad: 0,
        name,
    };

    write_response(out_buf, out_len, &info)
}
