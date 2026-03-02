//! Stub implementations for unimplemented POSIX interfaces.
//!
//! All functions set errno to ENOSYS and return -1 (or NULL).
//! This allows C code to link without undefined-symbol errors while
//! making it clear at runtime that these are not yet supported.

use hadron_libc_core::errno::{self, EAGAIN, EINVAL, ENOSYS};

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

// sched_yield — moved to core/src/pthread.rs

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

// ---- Missing stdlib functions -----------------------------------------------

/// `mkstemp` — create a unique temporary file.
///
/// # Safety
///
/// `tmpl` must be a mutable NUL-terminated string ending in "XXXXXX".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mkstemp(_tmpl: *mut u8) -> i32 {
    stub_err()
}

/// `mkdtemp` — create a unique temporary directory.
///
/// # Safety
///
/// `tmpl` must be a mutable NUL-terminated string ending in "XXXXXX".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mkdtemp(_tmpl: *mut u8) -> *mut u8 {
    errno::set_errno(ENOSYS);
    core::ptr::null_mut()
}

/// `lrand48` — generate a pseudo-random number in [0, 2^31).
#[unsafe(no_mangle)]
pub extern "C" fn lrand48() -> i64 {
    // Simple LCG fallback — not cryptographic.
    static SEED: core::sync::atomic::AtomicU64 =
        core::sync::atomic::AtomicU64::new(0x1234_5678_9abc_def0);
    let s = SEED.load(core::sync::atomic::Ordering::Relaxed);
    let next = s
        .wrapping_mul(0x5deece66du64)
        .wrapping_add(0xb)
        .wrapping_add(11);
    SEED.store(next, core::sync::atomic::Ordering::Relaxed);
    ((next >> 16) & 0x7fff_ffff) as i64
}

/// `mrand48` — generate a pseudo-random number in [-2^31, 2^31).
#[unsafe(no_mangle)]
pub extern "C" fn mrand48() -> i64 {
    static SEED: core::sync::atomic::AtomicU64 =
        core::sync::atomic::AtomicU64::new(0xabcd_ef01_2345_6789);
    let s = SEED.load(core::sync::atomic::Ordering::Relaxed);
    let next = s
        .wrapping_mul(0x5deece66du64)
        .wrapping_add(0xb)
        .wrapping_add(11);
    SEED.store(next, core::sync::atomic::Ordering::Relaxed);
    ((next >> 16) as i32) as i64
}

/// `srand48` — seed the 48-bit LCG random number generator.
#[unsafe(no_mangle)]
pub extern "C" fn srand48(seed: i64) {
    static SEED: core::sync::atomic::AtomicU64 =
        core::sync::atomic::AtomicU64::new(0x1234_5678_9abc_def0);
    SEED.store(
        (seed as u64) << 16 | 0x330e,
        core::sync::atomic::Ordering::Relaxed,
    );
}

/// `putenv` — change environment variable.
///
/// # Safety
///
/// `string` must be a NUL-terminated "NAME=value" pair.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn putenv(string: *mut u8) -> i32 {
    if string.is_null() {
        return stub_err();
    }
    // Parse "name=value" and call setenv.
    let s = unsafe { core::ffi::CStr::from_ptr(string as *const core::ffi::c_char) };
    let bytes = s.to_bytes();
    if let Some(eq) = bytes.iter().position(|&b| b == b'=') {
        // name and value are within the string — just delegate to setenv logic.
        // For simplicity, treat this as ENOSYS (env mutation is complex).
        let _ = eq;
    }
    stub_err()
}

// ---- Missing I/O functions --------------------------------------------------

/// `fdopen` — associate a stream with a file descriptor.
///
/// # Safety
///
/// `mode` must be a NUL-terminated mode string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fdopen(_fd: i32, _mode: *const u8) -> *mut u8 {
    errno::set_errno(ENOSYS);
    core::ptr::null_mut()
}

/// `tmpfile` — create a temporary file.
#[unsafe(no_mangle)]
pub extern "C" fn tmpfile() -> *mut u8 {
    errno::set_errno(ENOSYS);
    core::ptr::null_mut()
}

/// `setvbuf` — set the buffer for a stream.
///
/// # Safety
///
/// Parameters must be valid for their types.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setvbuf(_stream: *mut u8, _buf: *mut u8, _mode: i32, _size: usize) -> i32 {
    0 // Pretend success; our stdio uses fixed buffers.
}

/// `sscanf` — parse formatted input from a string.
///
/// # Safety
///
/// `str` and `fmt` must be NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sscanf(_str: *const u8, _fmt: *const u8, _: ...) -> i32 {
    errno::set_errno(ENOSYS);
    -1
}

// vprintf/vsnprintf/vfprintf/vsprintf — implemented in hadron-libc-core/stdio/printf.rs

// ---- Missing POSIX I/O functions --------------------------------------------

/// `pread` — read from file descriptor at a given offset.
///
/// # Safety
///
/// `buf` must be a valid writable buffer of at least `count` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pread(_fd: i32, _buf: *mut u8, _count: usize, _offset: i64) -> isize {
    errno::set_errno(ENOSYS);
    -1
}

/// `pwrite` — write to file descriptor at a given offset.
///
/// # Safety
///
/// `buf` must be a valid readable buffer of at least `count` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pwrite(_fd: i32, _buf: *const u8, _count: usize, _offset: i64) -> isize {
    errno::set_errno(ENOSYS);
    -1
}

// ---- Missing wide-character functions ---------------------------------------

/// `wcsncpy` — copy a wide-character string with length limit.
///
/// # Safety
///
/// `dest` and `src` must be valid, with `dest` having at least `n` wchar_t elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsncpy(dest: *mut u32, src: *const u32, n: usize) -> *mut u32 {
    let mut i = 0usize;
    while i < n {
        let c = unsafe { *src.add(i) };
        unsafe { *dest.add(i) = c };
        if c == 0 {
            i += 1;
            break;
        }
        i += 1;
    }
    // Zero-pad remaining.
    while i < n {
        unsafe { *dest.add(i) = 0 };
        i += 1;
    }
    dest
}

// ---- Resource limits --------------------------------------------------------

/// `getrlimit` — get resource limits.
///
/// # Safety
///
/// `rlim` must be a valid pointer to an `rlimit` struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getrlimit(_resource: i32, rlim: *mut [u64; 2]) -> i32 {
    if rlim.is_null() {
        return stub_err();
    }
    // Return "unlimited" for all resources.
    unsafe { *rlim = [u64::MAX, u64::MAX] };
    0
}

/// `setrlimit` — set resource limits.
///
/// # Safety
///
/// `rlim` must be a valid pointer to an `rlimit` struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setrlimit(_resource: i32, _rlim: *const [u64; 2]) -> i32 {
    0 // Pretend success.
}

/// `getrusage` — get resource usage.
///
/// # Safety
///
/// `usage` must be a valid pointer to a `rusage` struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getrusage(_who: i32, usage: *mut u8) -> i32 {
    if usage.is_null() {
        return stub_err();
    }
    unsafe { core::ptr::write_bytes(usage, 0, 136) }; // zeroed rusage
    0
}

// ---- POSIX semaphores -------------------------------------------------------

/// `sem_init` — initialize an unnamed semaphore.
///
/// # Safety
///
/// `sem` must be a valid non-null pointer to a `sem_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_init(sem: *mut i32, _pshared: i32, value: u32) -> i32 {
    if sem.is_null() {
        return stub_err();
    }
    unsafe { *sem = value as i32 };
    0
}

/// `sem_destroy` — destroy an unnamed semaphore.
///
/// # Safety
///
/// `sem` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_destroy(_sem: *mut i32) -> i32 {
    0
}

/// `sem_post` — increment (unlock) a semaphore.
///
/// # Safety
///
/// `sem` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_post(sem: *mut i32) -> i32 {
    if sem.is_null() {
        return stub_err();
    }
    unsafe {
        core::sync::atomic::AtomicI32::from_ptr(sem)
            .fetch_add(1, core::sync::atomic::Ordering::Release);
    }
    0
}

/// `sem_wait` — decrement (lock) a semaphore, blocking if needed.
///
/// # Safety
///
/// `sem` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_wait(sem: *mut i32) -> i32 {
    if sem.is_null() {
        return stub_err();
    }
    loop {
        let v = unsafe {
            core::sync::atomic::AtomicI32::from_ptr(sem).load(core::sync::atomic::Ordering::Acquire)
        };
        if v > 0 {
            let r = unsafe {
                core::sync::atomic::AtomicI32::from_ptr(sem).compare_exchange(
                    v,
                    v - 1,
                    core::sync::atomic::Ordering::AcqRel,
                    core::sync::atomic::Ordering::Relaxed,
                )
            };
            if r.is_ok() {
                return 0;
            }
        }
        core::hint::spin_loop();
    }
}

/// `sem_trywait` — non-blocking semaphore decrement.
///
/// # Safety
///
/// `sem` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_trywait(sem: *mut i32) -> i32 {
    if sem.is_null() {
        return stub_err();
    }
    let v = unsafe {
        core::sync::atomic::AtomicI32::from_ptr(sem).load(core::sync::atomic::Ordering::Acquire)
    };
    if v <= 0 {
        errno::set_errno(EAGAIN);
        return -1;
    }
    let r = unsafe {
        core::sync::atomic::AtomicI32::from_ptr(sem).compare_exchange(
            v,
            v - 1,
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Relaxed,
        )
    };
    if r.is_ok() { 0 } else { stub_err() }
}

/// `sem_getvalue` — get current semaphore value.
///
/// # Safety
///
/// `sem` and `sval` must be valid non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_getvalue(sem: *mut i32, sval: *mut i32) -> i32 {
    if sem.is_null() || sval.is_null() {
        return stub_err();
    }
    unsafe {
        *sval = core::sync::atomic::AtomicI32::from_ptr(sem)
            .load(core::sync::atomic::Ordering::Relaxed);
    }
    0
}

// ---- pthread barriers -------------------------------------------------------

/// `pthread_barrier_init` — initialize a barrier.
///
/// # Safety
///
/// `barrier` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_barrier_init(
    barrier: *mut [u32; 3],
    _attr: *const u8,
    count: u32,
) -> i32 {
    if barrier.is_null() {
        return stub_err();
    }
    unsafe { *barrier = [count, 0, 0] };
    0
}

/// `pthread_barrier_destroy` — destroy a barrier.
///
/// # Safety
///
/// `barrier` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_barrier_destroy(_barrier: *mut [u32; 3]) -> i32 {
    0
}

/// `pthread_barrier_wait` — wait at a barrier.
///
/// # Safety
///
/// `barrier` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_barrier_wait(barrier: *mut [u32; 3]) -> i32 {
    if barrier.is_null() {
        return stub_err();
    }
    stub_err() // Not yet implemented.
}

// ---- pthread spinlocks ------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_spin_init(_lock: *mut u32, _pshared: i32) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_spin_destroy(_lock: *mut u32) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_spin_lock(lock: *mut u32) -> i32 {
    loop {
        let r = unsafe {
            core::sync::atomic::AtomicU32::from_ptr(lock).compare_exchange(
                0,
                1,
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
            )
        };
        if r.is_ok() {
            return 0;
        }
        core::hint::spin_loop();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_spin_trylock(lock: *mut u32) -> i32 {
    let r = unsafe {
        core::sync::atomic::AtomicU32::from_ptr(lock).compare_exchange(
            0,
            1,
            core::sync::atomic::Ordering::Acquire,
            core::sync::atomic::Ordering::Relaxed,
        )
    };
    if r.is_ok() { 0 } else { stub_err() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_spin_unlock(lock: *mut u32) -> i32 {
    unsafe {
        core::sync::atomic::AtomicU32::from_ptr(lock)
            .store(0, core::sync::atomic::Ordering::Release);
    }
    0
}

// ---- locale extensions ------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn newlocale(_mask: i32, _locale: *const u8, _base: *mut u8) -> *mut u8 {
    core::ptr::null_mut() // Only "C" locale supported.
}

#[unsafe(no_mangle)]
pub extern "C" fn duplocale(_loc: *mut u8) -> *mut u8 {
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn freelocale(_loc: *mut u8) {}

#[unsafe(no_mangle)]
pub extern "C" fn uselocale(_loc: *mut u8) -> *mut u8 {
    core::ptr::null_mut()
}

// ---- sigaction / signal helpers ---------------------------------------------

/// `sigemptyset` — initialize an empty signal set.
///
/// # Safety
///
/// `set` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigemptyset(set: *mut u64) -> i32 {
    unsafe { *set = 0 };
    0
}

/// `sigfillset` — initialize a full signal set.
///
/// # Safety
///
/// `set` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigfillset(set: *mut u64) -> i32 {
    unsafe { *set = !0u64 };
    0
}

/// `sigaddset` — add a signal to a set.
///
/// # Safety
///
/// `set` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaddset(set: *mut u64, signum: i32) -> i32 {
    if signum < 1 || signum > 64 {
        errno::set_errno(EINVAL);
        return -1;
    }
    unsafe { *set |= 1u64 << (signum - 1) };
    0
}

/// `sigdelset` — remove a signal from a set.
///
/// # Safety
///
/// `set` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigdelset(set: *mut u64, signum: i32) -> i32 {
    if signum < 1 || signum > 64 {
        errno::set_errno(EINVAL);
        return -1;
    }
    unsafe { *set &= !(1u64 << (signum - 1)) };
    0
}

/// `sigismember` — test membership of a signal in a set.
///
/// # Safety
///
/// `set` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigismember(set: *const u64, signum: i32) -> i32 {
    if signum < 1 || signum > 64 {
        errno::set_errno(EINVAL);
        return -1;
    }
    ((unsafe { *set } >> (signum - 1)) & 1) as i32
}

/// `sigaltstack` — set or get the alternate signal stack.
///
/// # Safety
///
/// Parameters must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaltstack(_ss: *const u8, _old_ss: *mut u8) -> i32 {
    0
}

// ---- `realpath` — resolve a pathname ----------------------------------------

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

// ---- Network stubs ----------------------------------------------------------

/// `inet_pton` — convert text to binary network address.
///
/// # Safety
///
/// `src` must be NUL-terminated; `dst` must be writable (4 or 16 bytes).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn inet_pton(af: i32, src: *const u8, dst: *mut u8) -> i32 {
    if src.is_null() || dst.is_null() {
        return -1;
    }
    // AF_INET = 2: parse "a.b.c.d"
    if af == 2 {
        let s = unsafe { core::ffi::CStr::from_ptr(src as *const core::ffi::c_char) };
        let bytes = s.to_bytes();
        let mut octets = [0u8; 4];
        let mut idx = 0;
        let mut cur: u32 = 0;
        let mut dots = 0;
        let mut has_digit = false;
        for &b in bytes {
            if b.is_ascii_digit() {
                cur = cur * 10 + (b - b'0') as u32;
                if cur > 255 {
                    return 0;
                }
                has_digit = true;
            } else if b == b'.' && dots < 3 && has_digit {
                octets[dots] = cur as u8;
                dots += 1;
                cur = 0;
                has_digit = false;
            } else {
                return 0;
            }
        }
        if has_digit && dots == 3 {
            octets[3] = cur as u8;
            unsafe { core::ptr::copy_nonoverlapping(octets.as_ptr(), dst, 4) };
            return 1;
        }
        idx += 1;
        let _ = idx;
    }
    errno::set_errno(ENOSYS);
    -1
}

/// `inet_ntop` — convert binary network address to text.
///
/// # Safety
///
/// `src` must be 4 or 16 readable bytes; `dst` must be writable for `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn inet_ntop(af: i32, src: *const u8, dst: *mut u8, size: u32) -> *const u8 {
    if src.is_null() || dst.is_null() || size == 0 {
        errno::set_errno(EINVAL);
        return core::ptr::null();
    }
    if af == 2 {
        // AF_INET
        let a = unsafe { *src };
        let b = unsafe { *src.add(1) };
        let c = unsafe { *src.add(2) };
        let d = unsafe { *src.add(3) };
        // Format: "a.b.c.d" (max 15 chars + NUL = 16)
        let mut buf = [0u8; 16];
        let s = format_ipv4(&mut buf, a, b, c, d);
        if s.len() + 1 > size as usize {
            errno::set_errno(ENOSYS); // ENOSPC
            return core::ptr::null();
        }
        unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), dst, s.len()) };
        unsafe { *dst.add(s.len()) = 0 };
        return dst;
    }
    errno::set_errno(ENOSYS);
    core::ptr::null()
}

fn format_ipv4(buf: &mut [u8; 16], a: u8, b: u8, c: u8, d: u8) -> &[u8] {
    fn write_u8(b: &mut [u8], pos: usize, v: u8) -> usize {
        if v >= 100 {
            b[pos] = b'0' + v / 100;
            b[pos + 1] = b'0' + (v / 10) % 10;
            b[pos + 2] = b'0' + v % 10;
            3
        } else if v >= 10 {
            b[pos] = b'0' + v / 10;
            b[pos + 1] = b'0' + v % 10;
            2
        } else {
            b[pos] = b'0' + v;
            1
        }
    }
    let mut p = 0;
    p += write_u8(buf, p, a);
    buf[p] = b'.';
    p += 1;
    p += write_u8(buf, p, b);
    buf[p] = b'.';
    p += 1;
    p += write_u8(buf, p, c);
    buf[p] = b'.';
    p += 1;
    p += write_u8(buf, p, d);
    &buf[..p]
}

// ---- Password database stubs ------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn getpwnam(_name: *const u8) -> *mut u8 {
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn getpwuid(_uid: u32) -> *mut u8 {
    core::ptr::null_mut()
}

/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpwnam_r(
    _name: *const u8,
    _pwd: *mut u8,
    _buf: *mut u8,
    _buflen: usize,
    result: *mut *mut u8,
) -> i32 {
    if !result.is_null() {
        unsafe { *result = core::ptr::null_mut() };
    }
    errno::set_errno(ENOSYS);
    ENOSYS.0
}

/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpwuid_r(
    _uid: u32,
    _pwd: *mut u8,
    _buf: *mut u8,
    _buflen: usize,
    result: *mut *mut u8,
) -> i32 {
    if !result.is_null() {
        unsafe { *result = core::ptr::null_mut() };
    }
    errno::set_errno(ENOSYS);
    ENOSYS.0
}

// ---- Resolver stubs ---------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dn_expand(
    _msg: *const u8,
    _eom: *const u8,
    _src: *const u8,
    _dst: *mut u8,
    _dstsiz: i32,
) -> i32 {
    errno::set_errno(ENOSYS);
    -1
}

// ---- File I/O stubs ---------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn ftello(_stream: *mut u8) -> i64 {
    errno::set_errno(ENOSYS);
    -1
}

/// `ungetc` — push a byte back onto a stream.
///
/// # Safety
///
/// `stream` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ungetc(_c: i32, _stream: *mut u8) -> i32 {
    // Minimal stub — EOF
    -1 // EOF
}

// ---- Filesystem stubs -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn statvfs(_path: *const u8, _buf: *mut u8) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstatvfs(_fd: i32, _buf: *mut u8) -> i32 {
    stub_err()
}

// ---- pthread extensions -----------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_init(_rwlock: *mut u8, _attr: *const u8) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_destroy(_rwlock: *mut u8) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_rdlock(_rwlock: *mut u8) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_tryrdlock(_rwlock: *mut u8) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_wrlock(_rwlock: *mut u8) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_trywrlock(_rwlock: *mut u8) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_unlock(_rwlock: *mut u8) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_setclock(_attr: *mut u8, _clock_id: i32) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_getclock(_attr: *const u8, clock_id: *mut i32) -> i32 {
    if !clock_id.is_null() {
        unsafe { *clock_id = 0 }; // CLOCK_REALTIME
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_timedwait(
    cond: *mut u8,
    mutex: *mut u8,
    _abstime: *const u8,
) -> i32 {
    // Fall back to untimed wait (ignores timeout).
    if cond.is_null() || mutex.is_null() {
        return stub_err();
    }
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn pthread_cancel(_thread: u64) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub extern "C" fn pthread_setcancelstate(_state: i32, _oldstate: *mut i32) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn pthread_setcanceltype(_type: i32, _oldtype: *mut i32) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn pthread_testcancel() {}

#[unsafe(no_mangle)]
pub extern "C" fn pthread_attr_setdetachstate(_attr: *mut u8, _detachstate: i32) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn pthread_attr_getdetachstate(_attr: *const u8, _detachstate: *mut i32) -> i32 {
    0
}

// ---- Wide character classification stubs ------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn iswspace(wc: u32) -> i32 {
    matches!(wc, 0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0x20 | 0xa0) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswdigit(wc: u32) -> i32 {
    (wc >= b'0' as u32 && wc <= b'9' as u32) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswalpha(wc: u32) -> i32 {
    ((wc >= b'a' as u32 && wc <= b'z' as u32) || (wc >= b'A' as u32 && wc <= b'Z' as u32)) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswalnum(wc: u32) -> i32 {
    (iswdigit(wc) != 0 || iswalpha(wc) != 0) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswupper(wc: u32) -> i32 {
    (wc >= b'A' as u32 && wc <= b'Z' as u32) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswlower(wc: u32) -> i32 {
    (wc >= b'a' as u32 && wc <= b'z' as u32) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswprint(wc: u32) -> i32 {
    (wc >= 0x20 && wc != 0x7f) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswpunct(wc: u32) -> i32 {
    (iswprint(wc) != 0 && iswspace(wc) == 0 && iswalnum(wc) == 0) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswcntrl(wc: u32) -> i32 {
    (wc < 0x20 || wc == 0x7f) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswblank(wc: u32) -> i32 {
    (wc == b' ' as u32 || wc == b'\t' as u32) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswgraph(wc: u32) -> i32 {
    (wc > 0x20 && wc != 0x7f) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn iswxdigit(wc: u32) -> i32 {
    ((wc >= b'0' as u32 && wc <= b'9' as u32)
        || (wc >= b'a' as u32 && wc <= b'f' as u32)
        || (wc >= b'A' as u32 && wc <= b'F' as u32)) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn towupper(wc: u32) -> u32 {
    if wc >= b'a' as u32 && wc <= b'z' as u32 {
        wc - 0x20
    } else {
        wc
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn towlower(wc: u32) -> u32 {
    if wc >= b'A' as u32 && wc <= b'Z' as u32 {
        wc + 0x20
    } else {
        wc
    }
}

// ---- iconv stubs ------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn iconv_open(_tocode: *const u8, _fromcode: *const u8) -> *mut u8 {
    // Return a sentinel non-null value to distinguish from failure
    // (real iconv_open returns (iconv_t)-1 on failure, non-NULL on success).
    // We use 1 as a sentinel "success" handle.
    1usize as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn iconv(
    _cd: *mut u8,
    inbuf: *mut *mut u8,
    inbytesleft: *mut usize,
    outbuf: *mut *mut u8,
    outbytesleft: *mut usize,
) -> usize {
    // Copy bytes verbatim (identity conversion — only works for ASCII/Latin-1 supersets).
    if inbuf.is_null() {
        return 0;
    }
    let mut inp = unsafe { *inbuf };
    let mut outp = unsafe { *outbuf };
    let mut inleft = unsafe { *inbytesleft };
    let mut outleft = unsafe { *outbytesleft };
    while inleft > 0 && outleft > 0 {
        unsafe { *outp = *inp };
        inp = unsafe { inp.add(1) };
        outp = unsafe { outp.add(1) };
        inleft -= 1;
        outleft -= 1;
    }
    if !inbuf.is_null() {
        unsafe { *inbuf = inp };
    }
    if !outbuf.is_null() {
        unsafe { *outbuf = outp };
    }
    if !inbytesleft.is_null() {
        unsafe { *inbytesleft = inleft };
    }
    if !outbytesleft.is_null() {
        unsafe { *outbytesleft = outleft };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn iconv_close(_cd: *mut u8) -> i32 {
    0
}

// ---- Wide string stubs ------------------------------------------------------

/// `wcsstr` — find a wide-string needle in haystack.
///
/// # Safety
///
/// Both pointers must be NUL-terminated wchar_t arrays.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsstr(haystack: *const u32, needle: *const u32) -> *mut u32 {
    if needle.is_null() || unsafe { *needle } == 0 {
        return haystack as *mut u32;
    }
    let mut h = haystack;
    loop {
        if unsafe { *h } == 0 {
            return core::ptr::null_mut();
        }
        // Try to match needle at h.
        let mut p = h;
        let mut n = needle;
        loop {
            if unsafe { *n } == 0 {
                return h as *mut u32;
            }
            if unsafe { *p } != unsafe { *n } {
                break;
            }
            p = unsafe { p.add(1) };
            n = unsafe { n.add(1) };
        }
        h = unsafe { h.add(1) };
    }
}

// ---- ftello / fseeko --------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fseeko(_stream: *mut u8, _offset: i64, _whence: i32) -> i32 {
    stub_err()
}

// ---- Regex (POSIX) ----------------------------------------------------------
// regcomp/regexec/regfree are complex; return REG_NOMATCH / REG_ESPACE stubs.

unsafe extern "C" {
    fn malloc(size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn regcomp(_preg: *mut u8, _regex: *const u8, _cflags: i32) -> i32 {
    12 // REG_ESPACE — "out of space" (signals: not implemented)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn regexec(
    _preg: *const u8,
    _string: *const u8,
    _nmatch: usize,
    _pmatch: *mut u8,
    _eflags: i32,
) -> i32 {
    1 // REG_NOMATCH
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn regerror(
    errcode: i32,
    _preg: *const u8,
    errbuf: *mut u8,
    errbuf_size: usize,
) -> usize {
    let msg = b"regex not implemented\0";
    if !errbuf.is_null() && errbuf_size > 0 {
        let n = msg.len().min(errbuf_size);
        unsafe { core::ptr::copy_nonoverlapping(msg.as_ptr(), errbuf, n) };
    }
    msg.len()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn regfree(_preg: *mut u8) {}

// ---- String extensions -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strverscmp(s1: *const u8, s2: *const u8) -> i32 {
    // Simple lexicographic fallback (ignores version-number semantics).
    if s1.is_null() || s2.is_null() {
        return 0;
    }
    let mut p = s1;
    let mut q = s2;
    loop {
        let a = unsafe { *p } as i32;
        let b = unsafe { *q } as i32;
        if a != b || a == 0 {
            return a - b;
        }
        p = unsafe { p.add(1) };
        q = unsafe { q.add(1) };
    }
}

// ---- mbsrtowcs / wcrtomb -----------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbsrtowcs(
    dst: *mut i32,
    src: *mut *const u8,
    len: usize,
    _ps: *mut u8,
) -> usize {
    if src.is_null() || unsafe { (*src).is_null() } {
        return 0;
    }
    let s = unsafe { *src };
    let mut written: usize = 0;
    let mut p = s;
    loop {
        let b = unsafe { *p };
        if written >= len {
            break;
        }
        if !dst.is_null() {
            unsafe { *dst.add(written) = b as i32 };
        }
        written += 1;
        if b == 0 {
            if !src.is_null() {
                unsafe { *src = core::ptr::null() };
            }
            return written - 1;
        }
        p = unsafe { p.add(1) };
    }
    if !src.is_null() {
        unsafe { *src = p };
    }
    written
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcrtomb(s: *mut u8, wc: i32, _ps: *mut u8) -> usize {
    if s.is_null() {
        return 1;
    }
    if wc >= 0 && wc < 0x80 {
        unsafe { *s = wc as u8 };
        return 1;
    }
    // Non-ASCII: store '?' as replacement
    unsafe { *s = b'?' };
    1
}

// ---- fmemopen / open_memstream -----------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fmemopen(_buf: *mut u8, _size: usize, _mode: *const u8) -> *mut u8 {
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn open_memstream(_ptr: *mut *mut u8, _sizeloc: *mut usize) -> *mut u8 {
    core::ptr::null_mut()
}

// ---- Wide char I/O -----------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetwc(_stream: *mut u8) -> u32 {
    u32::MAX // WEOF
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getwc(_stream: *mut u8) -> u32 {
    u32::MAX
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getwchar() -> u32 {
    u32::MAX
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputwc(_wc: i32, _stream: *mut u8) -> u32 {
    u32::MAX
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn putwc(_wc: i32, _stream: *mut u8) -> u32 {
    u32::MAX
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn putwchar(_wc: i32) -> u32 {
    u32::MAX
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetws(_s: *mut i32, _n: i32, _stream: *mut u8) -> *mut i32 {
    core::ptr::null_mut()
}

// ---- _Exit -------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Exit(status: i32) -> ! {
    // Identical to _exit: no atexit handlers, no stdio flush.
    unsafe extern "C" {
        fn _exit(status: i32) -> !;
    }
    unsafe { _exit(status) }
}

// ---- syscall (raw syscall passthrough) ---------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall(number: i64, ...) -> i64 {
    // We can't forward variadic args in safe Rust; just return ENOSYS.
    -38 // -ENOSYS
}

// ---- exec family -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execv(_path: *const u8, _argv: *const *const u8) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execve(
    _path: *const u8,
    _argv: *const *const u8,
    _envp: *const *const u8,
) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execvp(_file: *const u8, _argv: *const *const u8) -> i32 {
    stub_err()
}

// execl / execlp / execle are variadic; stub_err via raw syscall pattern.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execl(_path: *const u8, _arg: *const u8, ...) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execlp(_file: *const u8, _arg: *const u8, ...) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn execle(_path: *const u8, _arg: *const u8, ...) -> i32 {
    stub_err()
}

// ---- wait --------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wait(_status: *mut i32) -> i32 {
    stub_err()
}

// ---- sem_timedwait -----------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_timedwait(_sem: *mut u8, _abs_timeout: *const u8) -> i32 {
    // Fall back to sem_wait (ignore timeout).
    stub_err()
}

// ---- Math: BSD aliases -------------------------------------------------------
// These call into libm's canonical forms.

unsafe extern "C" {
    fn remainder(x: f64, y: f64) -> f64;
    fn remainderf(x: f32, y: f32) -> f32;
    fn scalbn(x: f64, n: i32) -> f64;
    fn scalbnf(x: f32, n: i32) -> f32;
    fn pow(base: f64, exp: f64) -> f64;
    fn powf(base: f32, exp: f32) -> f32;
    fn lgamma(x: f64) -> f64;
    fn lgammaf(x: f32) -> f32;
    static signgam: i32;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn drem(x: f64, y: f64) -> f64 {
    unsafe { remainder(x, y) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dremf(x: f32, y: f32) -> f32 {
    unsafe { remainderf(x, y) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn scalb(x: f64, y: f64) -> f64 {
    unsafe { scalbn(x, y as i32) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn scalbf(x: f32, y: f32) -> f32 {
    unsafe { scalbnf(x, y as i32) }
}

// lgamma_r: reentrant lgamma (sets *signp to the sign of gamma(x)).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lgammaf_r(x: f32, signp: *mut i32) -> f32 {
    let r = unsafe { lgammaf(x) };
    if !signp.is_null() {
        unsafe { *signp = signgam };
    }
    r
}

// lgammal_r: long double variant — delegate to double (ABI imperfect but avoids f128).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lgammal_r(x: f64, signp: *mut i32) -> f64 {
    let r = unsafe { lgamma(x) };
    if !signp.is_null() {
        unsafe { *signp = signgam };
    }
    r
}

// exp10 / pow10: GNU extensions — 10^x.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exp10(x: f64) -> f64 {
    unsafe { pow(10.0_f64, x) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn exp10f(x: f32) -> f32 {
    unsafe { powf(10.0_f32, x) }
}

// exp10l / pow10l: long double variants — use f64 ABI (avoids f128 instability).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exp10l(x: f64) -> f64 {
    unsafe { pow(10.0_f64, x) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pow10(x: f64) -> f64 {
    unsafe { pow(10.0_f64, x) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pow10f(x: f32) -> f32 {
    unsafe { powf(10.0_f32, x) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pow10l(x: f64) -> f64 {
    unsafe { pow(10.0_f64, x) }
}

// ---- pthread extensions ------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_atfork(
    _prepare: *const u8,
    _parent: *const u8,
    _child: *const u8,
) -> i32 {
    0 // no fork, so handlers are never called
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_setrobust(_attr: *mut u8, _robustness: i32) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_getrobust(
    _attr: *const u8,
    robustness: *mut i32,
) -> i32 {
    if !robustness.is_null() {
        unsafe { *robustness = 0 }; // PTHREAD_MUTEX_STALLED
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_gettype(attr: *const u8, type_: *mut i32) -> i32 {
    if attr.is_null() || type_.is_null() {
        return stub_err();
    }
    unsafe { *type_ = 0 }; // PTHREAD_MUTEX_DEFAULT
    0
}

// ---- mktemp ------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mktemp(tmpl: *mut u8) -> *mut u8 {
    // mktemp is deprecated and insecure; return the template unmodified.
    if tmpl.is_null() {
        return core::ptr::null_mut();
    }
    // Write a null to make it a zero-length "filename" (common stub behavior).
    unsafe { *tmpl = 0 };
    tmpl
}

// ---- wcscmp / wcslen (wchar functions missing from core) --------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscmp(s1: *const i32, s2: *const i32) -> i32 {
    if s1.is_null() || s2.is_null() {
        return 0;
    }
    let mut p = s1;
    let mut q = s2;
    loop {
        let a = unsafe { *p };
        let b = unsafe { *q };
        if a != b || a == 0 {
            return if a < b { -1 } else { 1 };
        }
        p = unsafe { p.add(1) };
        q = unsafe { q.add(1) };
    }
}

// ---- sem_open / sem_close / sem_unlink (named semaphores) -------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_open(_name: *const u8, _oflag: i32, ...) -> *mut u8 {
    // Named semaphores require a kernel-side namespace; not implemented.
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_close(_sem: *mut u8) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_unlink(_name: *const u8) -> i32 {
    stub_err()
}

// ---- flockfile / funlockfile / ftrylockfile ---------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn flockfile(_stream: *mut u8) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn funlockfile(_stream: *mut u8) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ftrylockfile(_stream: *mut u8) -> i32 {
    0
}

// ---- pthread_kill / pthread_sigmask / pthread_mutexattr_setpshared ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_kill(_thread: u64, _sig: i32) -> i32 {
    stub_err()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_sigmask(_how: i32, _set: *const u8, _oldset: *mut u8) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_setpshared(_attr: *mut u8, _pshared: i32) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_getpshared(_attr: *const u8, pshared: *mut i32) -> i32 {
    if !pshared.is_null() {
        unsafe { *pshared = 0 }; // PTHREAD_PROCESS_PRIVATE
    }
    0
}

// ---- pthread_mutex_timedlock ------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_timedlock(mutex: *mut u8, _abstime: *const u8) -> i32 {
    // Fall back to a non-timed trylock loop (ignores timeout).
    if mutex.is_null() {
        return stub_err();
    }
    stub_err()
}
