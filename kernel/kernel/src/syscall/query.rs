//! System query syscall handler.
//!
//! Implements `sys_query`, which returns typed `#[repr(C)]` structs for
//! system information queries, replacing the text-based procfs approach.

use core::mem::size_of;

use crate::mm::PAGE_SIZE;
use crate::syscall::userptr::is_kernel_caller;
use crate::syscall::{
    CpuInfo, EFAULT, EINVAL, KernelVersionInfo, MemoryInfo, ProcessInfo, QUERY_CPU_INFO,
    QUERY_KERNEL_VERSION, QUERY_MEMORY, QUERY_PROCESSES, QUERY_UPTIME, QUERY_VMAPS, UptimeInfo,
    VmapEntry,
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
        QUERY_PROCESSES => query_processes(out_buf, out_len),
        QUERY_VMAPS => query_vmaps(out_buf, out_len),
        QUERY_CPU_INFO => query_cpu_info(out_buf, out_len),
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
        crate::mm::pmm::with(|pmm| (pmm.total_frames(), pmm.free_frames()));

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
        uptime_ns: crate::time::Time::boot_nanos(),
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

/// Handle `QUERY_PROCESSES`: return process table statistics.
#[expect(clippy::cast_possible_truncation, reason = "process count fits in u32")]
fn query_processes(out_buf: usize, out_len: usize) -> isize {
    let info = ProcessInfo {
        count: crate::proc::ProcessTable::count() as u32,
        _pad: 0,
    };

    write_response(out_buf, out_len, &info)
}

/// Handle `QUERY_VMAPS`: return the calling process's virtual memory map.
///
/// Writes an array of [`VmapEntry`] structs into `out_buf`. Returns the number
/// of bytes written (always a multiple of `size_of::<VmapEntry>()`), or
/// `-EINVAL` if the buffer is too small for even one entry.
///
/// The `sub_id` argument is reserved for future pagination; pass `0`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "page_count * PAGE_SIZE fits in u64 for sane mmap sizes"
)]
fn query_vmaps(out_buf: usize, out_len: usize) -> isize {
    use crate::proc::{MappingKind, ProcessTable};

    // Collect mappings from the current process.
    let entries: alloc::vec::Vec<VmapEntry> = ProcessTable::with_current(|proc| {
        let mappings = proc.mmap_mappings.lock();
        mappings
            .iter()
            .map(|(&base, &kind)| {
                let (page_count, prot, name_bytes) = match kind {
                    MappingKind::Anonymous { page_count } => {
                        (page_count, 0x3u32, *b"anon\0\0\0\0\0\0\0\0\0\0\0\0")
                    }
                    MappingKind::Device { page_count } => {
                        (page_count, 0x3u32, *b"device\0\0\0\0\0\0\0\0\0\0")
                    }
                    MappingKind::Shared { page_count } => {
                        (page_count, 0x3u32, *b"shared\0\0\0\0\0\0\0\0\0\0")
                    }
                };
                let start = base;
                let end = base + (page_count as u64) * PAGE_SIZE as u64;
                VmapEntry {
                    start,
                    end,
                    prot,
                    _pad: 0,
                    name: name_bytes,
                }
            })
            .collect()
    });

    let entry_size = size_of::<VmapEntry>();
    let max_entries = out_len / entry_size;
    let n = entries.len().min(max_entries);

    // Write entries directly into the output buffer.
    for (i, entry) in entries.iter().take(n).enumerate() {
        let dst = out_buf + i * entry_size;
        if is_kernel_caller(dst) {
            // SAFETY: Kernel-mode callers pass a valid kernel-space pointer.
            unsafe { core::ptr::write(dst as *mut VmapEntry, *entry) };
        } else {
            use crate::syscall::userptr::UserPtr;
            let Ok(user_ptr) = UserPtr::<VmapEntry>::new(dst) else {
                return -EFAULT;
            };
            // SAFETY: UserPtr validated alignment and user-space bounds.
            unsafe { core::ptr::write(user_ptr.addr() as *mut VmapEntry, *entry) };
        }
    }

    #[expect(
        clippy::cast_possible_wrap,
        reason = "n * entry_size is small and won't wrap isize"
    )]
    let written = (n * entry_size) as isize;
    written
}

/// Handle `QUERY_CPU_INFO`: return CPU capability information.
///
/// Returns core count, feature flags (matching `CpuFeatures` bitfield), and
/// the CPUID brand model string.
fn query_cpu_info(out_buf: usize, out_len: usize) -> isize {
    // Read CPUID brand string from leaves 0x8000_0002..=0x8000_0004.
    let mut model = [0u8; 48];

    #[cfg(target_arch = "x86_64")]
    {
        use crate::arch::x86_64::cpuid::cpuid;
        let leaves = [0x8000_0002u32, 0x8000_0003, 0x8000_0004];
        for (i, &leaf) in leaves.iter().enumerate() {
            let res = cpuid(leaf);
            let base = i * 16;
            model[base..base + 4].copy_from_slice(&res.eax.to_le_bytes());
            model[base + 4..base + 8].copy_from_slice(&res.ebx.to_le_bytes());
            model[base + 8..base + 12].copy_from_slice(&res.ecx.to_le_bytes());
            model[base + 12..base + 16].copy_from_slice(&res.edx.to_le_bytes());
        }
    }

    let feature_flags = {
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::cpuid::cpu_features().bits()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            0u64
        }
    };

    let info = CpuInfo {
        core_count: crate::percpu::PerCpuState::cpu_count(),
        _pad: 0,
        feature_flags,
        model,
    };

    write_response(out_buf, out_len, &info)
}
