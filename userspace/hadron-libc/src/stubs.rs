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

/// `atoi` — convert string to integer.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atoi(s: *const u8) -> i32 {
    if s.is_null() {
        return 0;
    }
    let mut i = 0usize;
    // Skip whitespace.
    while unsafe { *s.add(i) } == b' ' || unsafe { *s.add(i) } == b'\t' {
        i += 1;
    }
    // Sign.
    let neg = unsafe { *s.add(i) } == b'-';
    if neg || unsafe { *s.add(i) } == b'+' {
        i += 1;
    }
    // Digits.
    let mut val: i32 = 0;
    while matches!(unsafe { *s.add(i) }, b'0'..=b'9') {
        val = val
            .wrapping_mul(10)
            .wrapping_add((unsafe { *s.add(i) } - b'0') as i32);
        i += 1;
    }
    if neg { -val } else { val }
}

/// `atol` — convert string to long integer.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atol(s: *const u8) -> i64 {
    if s.is_null() {
        return 0;
    }
    let mut i = 0usize;
    while unsafe { *s.add(i) } == b' ' || unsafe { *s.add(i) } == b'\t' {
        i += 1;
    }
    let neg = unsafe { *s.add(i) } == b'-';
    if neg || unsafe { *s.add(i) } == b'+' {
        i += 1;
    }
    let mut val: i64 = 0;
    while matches!(unsafe { *s.add(i) }, b'0'..=b'9') {
        val = val
            .wrapping_mul(10)
            .wrapping_add((unsafe { *s.add(i) } - b'0') as i64);
        i += 1;
    }
    if neg { -val } else { val }
}

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

/// `strtol` — convert string to long integer.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtol(s: *const u8, endptr: *mut *const u8, base: i32) -> i64 {
    if s.is_null() {
        if !endptr.is_null() {
            unsafe { *endptr = s };
        }
        return 0;
    }

    let mut i = 0usize;
    // Skip whitespace.
    while unsafe { *s.add(i) } == b' ' || unsafe { *s.add(i) } == b'\t' {
        i += 1;
    }

    // Sign.
    let neg = unsafe { *s.add(i) } == b'-';
    if neg || unsafe { *s.add(i) } == b'+' {
        i += 1;
    }

    // Determine base.
    let mut radix = base as i64;
    if radix == 0 {
        if unsafe { *s.add(i) } == b'0' {
            if unsafe { *s.add(i + 1) } == b'x' || unsafe { *s.add(i + 1) } == b'X' {
                radix = 16;
                i += 2;
            } else {
                radix = 8;
                i += 1;
            }
        } else {
            radix = 10;
        }
    } else if radix == 16 {
        if unsafe { *s.add(i) } == b'0'
            && (unsafe { *s.add(i + 1) } == b'x' || unsafe { *s.add(i + 1) } == b'X')
        {
            i += 2;
        }
    }

    let mut val: i64 = 0;
    loop {
        let ch = unsafe { *s.add(i) };
        let digit = match ch {
            b'0'..=b'9' => (ch - b'0') as i64,
            b'a'..=b'z' => (ch - b'a' + 10) as i64,
            b'A'..=b'Z' => (ch - b'A' + 10) as i64,
            _ => break,
        };
        if digit >= radix {
            break;
        }
        val = val.wrapping_mul(radix).wrapping_add(digit);
        i += 1;
    }

    if !endptr.is_null() {
        unsafe { *endptr = s.add(i) };
    }
    if neg { -val } else { val }
}

/// `strtoul` — convert string to unsigned long.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoul(s: *const u8, endptr: *mut *const u8, base: i32) -> u64 {
    // Simplified: delegate to strtol and cast.
    unsafe { strtol(s, endptr, base) as u64 }
}

/// `strtoll` — convert string to long long.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoll(s: *const u8, endptr: *mut *const u8, base: i32) -> i64 {
    unsafe { strtol(s, endptr, base) }
}

/// `strtoull` — convert string to unsigned long long.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoull(s: *const u8, endptr: *mut *const u8, base: i32) -> u64 {
    unsafe { strtoul(s, endptr, base) }
}

/// `strtoimax` — convert string to intmax_t.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoimax(s: *const u8, endptr: *mut *const u8, base: i32) -> i64 {
    unsafe { strtol(s, endptr, base) }
}

/// `strtoumax` — convert string to uintmax_t.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoumax(s: *const u8, endptr: *mut *const u8, base: i32) -> u64 {
    unsafe { strtoul(s, endptr, base) }
}

// ---- String utilities (strdup, strcasecmp, etc.) ----------------------------

/// `strdup` — duplicate a string.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strdup(s: *const u8) -> *mut u8 {
    if s.is_null() {
        return core::ptr::null_mut();
    }
    let len = unsafe { hadron_libc_core::string::strlen(s) };
    let p = unsafe { hadron_libc_core::alloc::malloc(len + 1) };
    if p.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { hadron_libc_core::string::memcpy(p, s, len + 1) };
    p
}

/// `strndup` — duplicate at most `n` bytes of a string.
///
/// # Safety
///
/// `s` must be a valid string of at least `n` bytes or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strndup(s: *const u8, n: usize) -> *mut u8 {
    if s.is_null() {
        return core::ptr::null_mut();
    }
    let len = unsafe { hadron_libc_core::string::strnlen(s, n) };
    let p = unsafe { hadron_libc_core::alloc::malloc(len + 1) };
    if p.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { hadron_libc_core::string::memcpy(p, s, len) };
    unsafe { *p.add(len) = 0 };
    p
}

/// `strcasecmp` — case-insensitive string comparison.
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcasecmp(s1: *const u8, s2: *const u8) -> i32 {
    let mut i = 0;
    loop {
        let a = unsafe { *s1.add(i) };
        let b = unsafe { *s2.add(i) };
        let la = if a >= b'A' && a <= b'Z' { a + 32 } else { a };
        let lb = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
        if la != lb || la == 0 {
            return (la as i32) - (lb as i32);
        }
        i += 1;
    }
}

/// `strncasecmp` — case-insensitive string comparison (bounded).
///
/// # Safety
///
/// Both strings must be valid for `n` bytes or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncasecmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let a = unsafe { *s1.add(i) };
        let b = unsafe { *s2.add(i) };
        let la = if a >= b'A' && a <= b'Z' { a + 32 } else { a };
        let lb = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
        if la != lb || la == 0 {
            return (la as i32) - (lb as i32);
        }
        i += 1;
    }
    0
}

/// `strspn` — length of prefix consisting of accepted bytes.
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strspn(s: *const u8, accept: *const u8) -> usize {
    let mut count = 0;
    loop {
        let ch = unsafe { *s.add(count) };
        if ch == 0 {
            break;
        }
        let mut found = false;
        let mut j = 0;
        loop {
            let a = unsafe { *accept.add(j) };
            if a == 0 {
                break;
            }
            if a == ch {
                found = true;
                break;
            }
            j += 1;
        }
        if !found {
            break;
        }
        count += 1;
    }
    count
}

/// `strcspn` — length of prefix not containing rejected bytes.
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcspn(s: *const u8, reject: *const u8) -> usize {
    let mut count = 0;
    loop {
        let ch = unsafe { *s.add(count) };
        if ch == 0 {
            break;
        }
        let mut j = 0;
        loop {
            let r = unsafe { *reject.add(j) };
            if r == 0 {
                break;
            }
            if r == ch {
                return count;
            }
            j += 1;
        }
        count += 1;
    }
    count
}

/// `strpbrk` — find first occurrence of any accepted byte.
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strpbrk(s: *const u8, accept: *const u8) -> *const u8 {
    let pos = unsafe { strcspn(s, accept) };
    if unsafe { *s.add(pos) } == 0 {
        core::ptr::null()
    } else {
        unsafe { s.add(pos) }
    }
}

/// `strtok_r` — reentrant string tokenizer.
///
/// # Safety
///
/// `str` (or `*saveptr`) must be valid NUL-terminated strings. `delim` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok_r(
    str: *mut u8,
    delim: *const u8,
    saveptr: *mut *mut u8,
) -> *mut u8 {
    let mut s = if str.is_null() {
        unsafe { *saveptr }
    } else {
        str
    };
    if s.is_null() {
        return core::ptr::null_mut();
    }
    // Skip leading delimiters.
    s = unsafe { s.add(strspn(s, delim)) };
    if unsafe { *s } == 0 {
        unsafe { *saveptr = core::ptr::null_mut() };
        return core::ptr::null_mut();
    }
    let token = s;
    s = unsafe { s.add(strcspn(s, delim)) };
    if unsafe { *s } != 0 {
        unsafe { *s = 0 };
        unsafe { *saveptr = s.add(1) };
    } else {
        unsafe { *saveptr = core::ptr::null_mut() };
    }
    token
}

static mut STRTOK_SAVE: *mut u8 = core::ptr::null_mut();

/// `strtok` — string tokenizer (non-reentrant).
///
/// # Safety
///
/// `str` must be valid NUL-terminated. `delim` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok(str: *mut u8, delim: *const u8) -> *mut u8 {
    unsafe { strtok_r(str, delim, core::ptr::addr_of_mut!(STRTOK_SAVE)) }
}

// ---- qsort / bsearch -------------------------------------------------------

/// `qsort` — sort an array.
///
/// # Safety
///
/// `base` must be valid for `nmemb * size` bytes. `compar` must be a valid function pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qsort(
    base: *mut u8,
    nmemb: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
) {
    // Simple insertion sort — adequate for Mesa's small arrays.
    if nmemb <= 1 || size == 0 {
        return;
    }
    // Stack-allocate a temp buffer for elements up to 256 bytes.
    // For larger elements, swap byte-by-byte in place.
    let mut tmp = [0u8; 256];
    for i in 1..nmemb {
        let mut j = i;
        while j > 0 {
            let a = unsafe { base.add(j.wrapping_sub(1).wrapping_mul(size)) };
            let b = unsafe { base.add(j.wrapping_mul(size)) };
            if unsafe { compar(a, b) } <= 0 {
                break;
            }
            // Swap elements.
            if size <= 256 {
                unsafe {
                    core::ptr::copy_nonoverlapping(a, tmp.as_mut_ptr(), size);
                    core::ptr::copy_nonoverlapping(b, a, size);
                    core::ptr::copy_nonoverlapping(tmp.as_ptr(), b, size);
                }
            } else {
                for k in 0..size {
                    unsafe {
                        let t = *a.add(k);
                        *a.add(k) = *b.add(k);
                        *b.add(k) = t;
                    }
                }
            }
            j -= 1;
        }
    }
}

/// `bsearch` — binary search a sorted array.
///
/// # Safety
///
/// `base` must be valid for `nmemb * size` bytes. Array must be sorted per `compar`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsearch(
    key: *const u8,
    base: *const u8,
    nmemb: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
) -> *const u8 {
    let mut lo = 0usize;
    let mut hi = nmemb;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let elem = unsafe { base.add(mid.wrapping_mul(size)) };
        let cmp = unsafe { compar(key, elem) };
        if cmp == 0 {
            return elem;
        } else if cmp < 0 {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    core::ptr::null()
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

// ---- Float parsing stubs (minimal for compilation) --------------------------

/// `strtod` — convert string to double.
///
/// Minimal implementation handling integer parts and simple decimals.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtod(s: *const u8, endptr: *mut *const u8) -> f64 {
    if s.is_null() {
        if !endptr.is_null() {
            unsafe { *endptr = s };
        }
        return 0.0;
    }
    let mut i = 0usize;
    // Skip whitespace.
    while matches!(unsafe { *s.add(i) }, b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    // Sign.
    let neg = unsafe { *s.add(i) } == b'-';
    if neg || unsafe { *s.add(i) } == b'+' {
        i += 1;
    }
    // Integer part.
    let mut val: f64 = 0.0;
    while matches!(unsafe { *s.add(i) }, b'0'..=b'9') {
        val = val * 10.0 + (unsafe { *s.add(i) } - b'0') as f64;
        i += 1;
    }
    // Fractional part.
    if unsafe { *s.add(i) } == b'.' {
        i += 1;
        let mut frac: f64 = 0.1;
        while matches!(unsafe { *s.add(i) }, b'0'..=b'9') {
            val += (unsafe { *s.add(i) } - b'0') as f64 * frac;
            frac *= 0.1;
            i += 1;
        }
    }
    // Exponent.
    if matches!(unsafe { *s.add(i) }, b'e' | b'E') {
        i += 1;
        let exp_neg = unsafe { *s.add(i) } == b'-';
        if exp_neg || unsafe { *s.add(i) } == b'+' {
            i += 1;
        }
        let mut exp: i32 = 0;
        while matches!(unsafe { *s.add(i) }, b'0'..=b'9') {
            exp = exp * 10 + (unsafe { *s.add(i) } - b'0') as i32;
            i += 1;
        }
        // Compute 10^exp using repeated multiplication.
        let mut mult: f64 = 1.0;
        for _ in 0..exp {
            mult *= 10.0;
        }
        if exp_neg {
            val /= mult;
        } else {
            val *= mult;
        }
    }
    if !endptr.is_null() {
        unsafe { *endptr = s.add(i) };
    }
    if neg { -val } else { val }
}

/// `strtof` — convert string to float.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtof(s: *const u8, endptr: *mut *const u8) -> f32 {
    unsafe { strtod(s, endptr) as f32 }
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
