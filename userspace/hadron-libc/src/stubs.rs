//! Stub implementations for unimplemented POSIX interfaces.
//!
//! All functions set errno to ENOSYS and return -1 (or NULL).
//! This allows C code to link without undefined-symbol errors while
//! making it clear at runtime that these are not yet supported.

use hadron_libc_core::errno::{self, EINVAL, ENOSYS};

fn stub_err() -> i32 {
    errno::set_errno(ENOSYS);
    -1
}

// ---- Sockets ----------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn socket(_domain: i32, _ty: i32, _proto: i32) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn bind(_fd: i32, _addr: *const u8, _addrlen: u32) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn listen(_fd: i32, _backlog: i32) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn accept(_fd: i32, _addr: *mut u8, _addrlen: *mut u32) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn connect(_fd: i32, _addr: *const u8, _addrlen: u32) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn send(_fd: i32, _buf: *const u8, _len: usize, _flags: i32) -> isize {
    errno::set_errno(ENOSYS);
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn recv(_fd: i32, _buf: *mut u8, _len: usize, _flags: i32) -> isize {
    errno::set_errno(ENOSYS);
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn sendto(
    _fd: i32,
    _buf: *const u8,
    _len: usize,
    _flags: i32,
    _dest: *const u8,
    _addrlen: u32,
) -> isize {
    errno::set_errno(ENOSYS);
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn recvfrom(
    _fd: i32,
    _buf: *mut u8,
    _len: usize,
    _flags: i32,
    _src: *mut u8,
    _addrlen: *mut u32,
) -> isize {
    errno::set_errno(ENOSYS);
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn setsockopt(
    _fd: i32,
    _level: i32,
    _optname: i32,
    _optval: *const u8,
    _optlen: u32,
) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn getsockopt(
    _fd: i32,
    _level: i32,
    _optname: i32,
    _optval: *mut u8,
    _optlen: *mut u32,
) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn shutdown(_fd: i32, _how: i32) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn getpeername(_fd: i32, _addr: *mut u8, _addrlen: *mut u32) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn getsockname(_fd: i32, _addr: *mut u8, _addrlen: *mut u32) -> i32 {
    stub_err()
}

// ---- Termios ----------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn tcgetattr(_fd: i32, _termios: *mut u8) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn tcsetattr(_fd: i32, _action: i32, _termios: *const u8) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn cfgetispeed(_termios: *const u8) -> u32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn cfgetospeed(_termios: *const u8) -> u32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn cfsetispeed(_termios: *mut u8, _speed: u32) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn cfsetospeed(_termios: *mut u8, _speed: u32) -> i32 {
    stub_err()
}

// ---- Select -----------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn select(
    _nfds: i32,
    _readfds: *mut u8,
    _writefds: *mut u8,
    _exceptfds: *mut u8,
    _timeout: *mut u8,
) -> i32 {
    stub_err()
}

// ---- Misc stubs -------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn fork() -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn vfork() -> i32 {
    stub_err()
}

/// `abort()` — abnormal process termination.
#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
    // Try to raise SIGABRT first.
    let _ = hadron_libc_core::sys::sys_kill(
        hadron_libc_core::sys::sys_getpid(),
        hadron_libc_core::flags::SIGABRT as usize,
    );
    // If signal delivery didn't terminate us, force exit.
    hadron_libc_core::sys::sys_exit(134) // 128 + SIGABRT(6)
}

// atoi, atol, atoll, strtol, strtoul, strtoll, strtoull, strtoimax,
// strtoumax, strtof, strtod, strtold are implemented in hadron-libc-core::conv.

/// `abs` — absolute value of an integer.
#[unsafe(no_mangle)]
pub extern "C" fn abs(x: i32) -> i32 {
    if x < 0 { -x } else { x }
}

/// `labs` — absolute value of a long integer.
#[unsafe(no_mangle)]
pub extern "C" fn labs(x: i64) -> i64 {
    if x < 0 { -x } else { x }
}

// ---- System configuration ---------------------------------------------------

const SC_PAGE_SIZE: i32 = 30;
const SC_NPROCESSORS_ONLN: i32 = 84;
const SC_NPROCESSORS_CONF: i32 = 83;
const SC_PHYS_PAGES: i32 = 85;
const PAGE_SIZE: i64 = 4096;

/// `sysconf` — get configurable system variables.
#[unsafe(no_mangle)]
pub extern "C" fn sysconf(name: i32) -> i64 {
    match name {
        SC_PAGE_SIZE => PAGE_SIZE,
        SC_NPROCESSORS_ONLN | SC_NPROCESSORS_CONF => 1,
        // ~64 MiB worth of pages — reasonable for early Hadron
        SC_PHYS_PAGES => 16384,
        _ => -1,
    }
}

/// `getpagesize` — get the system page size.
#[unsafe(no_mangle)]
pub extern "C" fn getpagesize() -> i32 {
    PAGE_SIZE as i32
}

// ---- uname ------------------------------------------------------------------

/// Hadron utsname structure layout — must match sys/utsname.h (65-byte fields).
const UTSNAME_LENGTH: usize = 65;

/// Helper to copy a static string into a fixed-size utsname field.
///
/// # Safety
///
/// `dst` must be valid for `UTSNAME_LENGTH` bytes.
unsafe fn fill_utsname_field(dst: *mut u8, src: &[u8]) {
    let len = if src.len() < UTSNAME_LENGTH - 1 {
        src.len()
    } else {
        UTSNAME_LENGTH - 1
    };
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), dst, len);
        // NUL-terminate.
        *dst.add(len) = 0;
    }
}

/// `uname` — get system identification.
///
/// # Safety
///
/// `buf` must point to a valid `struct utsname` (5 × 65 bytes = 325 bytes).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uname(buf: *mut u8) -> i32 {
    if buf.is_null() {
        errno::set_errno(EINVAL);
        return -1;
    }
    unsafe {
        fill_utsname_field(buf, b"Hadron");
        fill_utsname_field(buf.add(UTSNAME_LENGTH), b"hadron");
        fill_utsname_field(buf.add(UTSNAME_LENGTH * 2), b"0.1.0");
        fill_utsname_field(buf.add(UTSNAME_LENGTH * 3), b"0.1.0");
        fill_utsname_field(buf.add(UTSNAME_LENGTH * 4), b"x86_64");
    }
    0
}

// ---- Dynamic linking stubs --------------------------------------------------

/// `dlopen` — stub (no dynamic linker on Hadron).
#[unsafe(no_mangle)]
pub extern "C" fn dlopen(_filename: *const u8, _flags: i32) -> *mut u8 {
    core::ptr::null_mut()
}

/// `dlsym` — stub (no dynamic linker on Hadron).
#[unsafe(no_mangle)]
pub extern "C" fn dlsym(_handle: *mut u8, _symbol: *const u8) -> *mut u8 {
    core::ptr::null_mut()
}

/// `dlclose` — stub.
#[unsafe(no_mangle)]
pub extern "C" fn dlclose(_handle: *mut u8) -> i32 {
    0
}

static DLERROR_MSG: &[u8] = b"dynamic linking not supported on Hadron\0";

/// `dlerror` — return static error message.
#[unsafe(no_mangle)]
pub extern "C" fn dlerror() -> *const u8 {
    DLERROR_MSG.as_ptr()
}

// ---- Scheduling stubs -------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn sched_getaffinity(_pid: i32, cpusetsize: usize, mask: *mut u8) -> i32 {
    if mask.is_null() || cpusetsize == 0 {
        errno::set_errno(ENOSYS);
        return -1;
    }
    // Report 1 CPU available.
    unsafe {
        hadron_libc_core::string::memset(mask, 0, cpusetsize);
        *mask = 1; // CPU 0 set
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_setaffinity(_pid: i32, _cpusetsize: usize, _mask: *const u8) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_yield() -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_get_priority_max(_policy: i32) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn sched_get_priority_min(_policy: i32) -> i32 {
    0
}

/// `__sched_cpucount` — count set bits in a CPU set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sched_cpucount(setsize: usize, set: *const u8) -> i32 {
    let mut count = 0i32;
    for i in 0..setsize {
        let byte = unsafe { *set.add(i) };
        count += byte.count_ones() as i32;
    }
    count
}

// ---- Misc stubs needed by Mesa ----------------------------------------------

/// `access` — check file accessibility (stub).
#[unsafe(no_mangle)]
pub extern "C" fn access(_path: *const u8, _mode: i32) -> i32 {
    // Allow access checks to succeed for now.
    0
}

/// `getuid` — get user ID.
#[unsafe(no_mangle)]
pub extern "C" fn getuid() -> u32 {
    0 // root
}

/// `geteuid` — get effective user ID.
#[unsafe(no_mangle)]
pub extern "C" fn geteuid() -> u32 {
    0 // root
}

/// `getgid` — get group ID.
#[unsafe(no_mangle)]
pub extern "C" fn getgid() -> u32 {
    0
}

/// `getegid` — get effective group ID.
#[unsafe(no_mangle)]
pub extern "C" fn getegid() -> u32 {
    0
}

/// `flock` — stub for file locking.
#[unsafe(no_mangle)]
pub extern "C" fn flock(_fd: i32, _operation: i32) -> i32 {
    0
}

/// `realpath` — resolve a pathname.
///
/// # Safety
///
/// `path` must be NUL-terminated. If `resolved` is null, allocates memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realpath(path: *const u8, resolved: *mut u8) -> *mut u8 {
    if path.is_null() {
        errno::set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    // Simplified: just copy path as-is (no symlink resolution on Hadron yet).
    let len = unsafe { hadron_libc_core::string::strlen(path) };
    let buf = if resolved.is_null() {
        unsafe { hadron_libc_core::alloc::malloc(len + 1) }
    } else {
        resolved
    };
    if buf.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { hadron_libc_core::string::memcpy(buf, path, len + 1) };
    buf
}
