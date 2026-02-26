//! Stub implementations for unimplemented POSIX interfaces.
//!
//! All functions set errno to ENOSYS and return -1 (or NULL).
//! This allows C code to link without undefined-symbol errors while
//! making it clear at runtime that these are not yet supported.

use hadron_libc_core::errno::{self, ENOSYS};

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
