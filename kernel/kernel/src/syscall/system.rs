//! System syscalls: `debug_log` and `query`.

use super::validate::UserSlice;
/// `SYS_DEBUG_LOG(buf_ptr, len)` — write a user buffer to the serial port.
pub fn sys_debug_log(buf_ptr: usize, len: usize) -> isize {
    let slice = match UserSlice::new(buf_ptr, len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: The user buffer has been range-validated. In Phase 1/2a the
    // process shares the kernel's CR3, so user pages are always mapped.
    let data = unsafe { slice.read_to_vec() };

    let com1 = crate::arch::x86_64::Port::<u8>::new(0x3F8);
    for &b in &data {
        // SAFETY: Port 0x3F8 is the standard COM1 data register.
        unsafe { com1.write(b) };
    }

    0
}

/// `SYS_QUERY(type, sub_id, buf_ptr, len)` — query system information.
pub fn sys_query(query_type: usize, _sub_id: usize, buf_ptr: usize, buf_len: usize) -> isize {
    match query_type as u32 {
        hadron_syscall::QUERY_MEMORY => query_memory(buf_ptr, buf_len),
        hadron_syscall::QUERY_UPTIME => query_uptime(buf_ptr, buf_len),
        hadron_syscall::QUERY_KERNEL_VERSION => query_kernel_version(buf_ptr, buf_len),
        _ => -hadron_syscall::EINVAL,
    }
}

fn query_memory(buf_ptr: usize, buf_len: usize) -> isize {
    if buf_len < core::mem::size_of::<hadron_syscall::MemoryInfo>() {
        return -hadron_syscall::EINVAL;
    }
    let slice = match UserSlice::new(buf_ptr, buf_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let info = hadron_syscall::MemoryInfo {
        total_bytes: 0, // Phase 2a: stub values
        free_bytes: 0,
        kernel_bytes: 0,
    };
    let bytes =
        // SAFETY: MemoryInfo is repr(C) with no padding invariants.
        unsafe { core::slice::from_raw_parts(&info as *const _ as *const u8, core::mem::size_of_val(&info)) };
    // SAFETY: User buffer was validated.
    unsafe { slice.write_from_slice(bytes) };
    0
}

fn query_uptime(buf_ptr: usize, buf_len: usize) -> isize {
    if buf_len < core::mem::size_of::<hadron_syscall::UptimeInfo>() {
        return -hadron_syscall::EINVAL;
    }
    let slice = match UserSlice::new(buf_ptr, buf_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // timer_ticks() returns monotonic tick count; convert to approximate ns.
    // Phase 2a stub: 1 tick ≈ 1 ms (PIT frequency).
    let ticks = crate::time::Time::timer_ticks();
    let total_ms = ticks;

    let info = hadron_syscall::UptimeInfo {
        secs: total_ms / 1000,
        nanos: (total_ms % 1000) * 1_000_000,
    };
    let bytes =
        // SAFETY: UptimeInfo is repr(C).
        unsafe { core::slice::from_raw_parts(&info as *const _ as *const u8, core::mem::size_of_val(&info)) };
    // SAFETY: User buffer was validated.
    unsafe { slice.write_from_slice(bytes) };
    0
}

fn query_kernel_version(buf_ptr: usize, buf_len: usize) -> isize {
    if buf_len < core::mem::size_of::<hadron_syscall::KernelVersionInfo>() {
        return -hadron_syscall::EINVAL;
    }
    let slice = match UserSlice::new(buf_ptr, buf_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let info = hadron_syscall::KernelVersionInfo {
        major: 0,
        minor: 1,
        patch: 0,
        _pad: 0,
    };
    let bytes =
        // SAFETY: KernelVersionInfo is repr(C).
        unsafe { core::slice::from_raw_parts(&info as *const _ as *const u8, core::mem::size_of_val(&info)) };
    // SAFETY: User buffer was validated.
    unsafe { slice.write_from_slice(bytes) };
    0
}
