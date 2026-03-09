//! Time functions.
//!
//! POSIX functions: `clock_gettime`, `nanosleep`, `time`, `sleep`, `usleep`.

use crate::errno;
use crate::sys;

/// `struct timespec` — POSIX time representation.
#[repr(C)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

/// POSIX `CLOCK_REALTIME`.
pub const CLOCK_REALTIME: i32 = 0;
/// POSIX `CLOCK_MONOTONIC`.
pub const CLOCK_MONOTONIC: i32 = 1;

/// Get the time of a specified clock.
///
/// # Safety
///
/// `tp` must be a valid pointer to a `Timespec`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clock_gettime(clockid: i32, tp: *mut Timespec) -> i32 {
    if tp.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    match sys::sys_clock_gettime(clockid as usize, tp.cast()) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Suspend execution for an interval.
///
/// # Safety
///
/// `req` must be a valid pointer to a `Timespec`. `rem` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nanosleep(req: *const Timespec, rem: *mut Timespec) -> i32 {
    if req.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    match sys::sys_nanosleep(req.cast(), rem.cast::<u8>()) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Return the current time in seconds since the epoch.
///
/// # Safety
///
/// `tloc` may be null; if non-null, the time is stored there.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn time(tloc: *mut i64) -> i64 {
    let mut ts = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { clock_gettime(CLOCK_REALTIME, &raw mut ts) } != 0 {
        return -1;
    }
    if !tloc.is_null() {
        // SAFETY: Caller guarantees tloc is valid.
        unsafe { *tloc = ts.tv_sec };
    }
    ts.tv_sec
}

/// Sleep for the specified number of seconds.
#[unsafe(no_mangle)]
pub extern "C" fn sleep(seconds: u32) -> u32 {
    let req = Timespec {
        tv_sec: seconds as i64,
        tv_nsec: 0,
    };
    let mut rem = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { nanosleep(&req, &raw mut rem) } != 0 {
        rem.tv_sec as u32
    } else {
        0
    }
}

/// Sleep for the specified number of microseconds.
#[unsafe(no_mangle)]
pub extern "C" fn usleep(usec: u32) -> i32 {
    let req = Timespec {
        tv_sec: (usec / 1_000_000) as i64,
        tv_nsec: ((usec % 1_000_000) as i64) * 1000,
    };
    unsafe { nanosleep(&req, core::ptr::null_mut()) }
}
