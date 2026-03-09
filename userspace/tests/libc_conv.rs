//! utest: POSIX string-to-number conversion regression tests.
//!
//! Covers: `atoi`, `atol`, `strtol` (bases 10/8/16/0), `strtoul`,
//! `strtod`, `strtof`, `strtoimax`.

#![no_std]
#![no_main]

// Force hadron_libc_core to be linked so its #[no_mangle] symbols (atoi,
// strtol, …) are available to the `extern "C"` declarations below.
extern crate hadron_libc_core;

use hadron_utest::utest_main;

utest_main!(
    test_atoi,
    test_atol,
    test_strtol_base10,
    test_strtol_base8,
    test_strtol_base16,
    test_strtol_base0,
    test_strtoul,
    test_strtod,
    test_strtof,
    test_strtoimax,
);

// ── extern declarations ───────────────────────────────────────────────────────

unsafe extern "C" {
    fn atoi(s: *const u8) -> i32;
    fn atol(s: *const u8) -> i64;
    fn strtol(s: *const u8, endptr: *mut *mut u8, base: i32) -> i64;
    fn strtoul(s: *const u8, endptr: *mut *mut u8, base: i32) -> u64;
    fn strtod(s: *const u8, endptr: *mut *mut u8) -> f64;
    fn strtof(s: *const u8, endptr: *mut *mut u8) -> f32;
    fn strtoimax(s: *const u8, endptr: *mut *mut u8, base: i32) -> i64;
}

// ── tests ─────────────────────────────────────────────────────────────────────

fn test_atoi() {
    // SAFETY: All arguments are valid NUL-terminated C strings.
    unsafe {
        assert_eq!(atoi(b"42\0".as_ptr()), 42);
        assert_eq!(atoi(b"-7\0".as_ptr()), -7);
        assert_eq!(atoi(b"0\0".as_ptr()), 0);
        assert_eq!(atoi(b"  123\0".as_ptr()), 123); // leading whitespace
        assert_eq!(atoi(b"99abc\0".as_ptr()), 99); // stops at non-digit
    }
}

fn test_atol() {
    // SAFETY: All arguments are valid NUL-terminated C strings.
    unsafe {
        assert_eq!(atol(b"1000000\0".as_ptr()), 1_000_000i64);
        assert_eq!(atol(b"-1\0".as_ptr()), -1i64);
        assert_eq!(atol(b"0\0".as_ptr()), 0i64);
    }
}

fn test_strtol_base10() {
    let mut end: *mut u8 = core::ptr::null_mut();
    // SAFETY: Input is a valid C string; endptr is a valid out-pointer.
    unsafe {
        let v = strtol(b"123\0".as_ptr(), &mut end, 10);
        assert_eq!(v, 123);

        let v2 = strtol(b"-456\0".as_ptr(), &mut end, 10);
        assert_eq!(v2, -456);

        let v3 = strtol(b"  +789abc\0".as_ptr(), &mut end, 10);
        assert_eq!(v3, 789);
        // end should point at 'a'
        assert_eq!(*end, b'a');
    }
}

fn test_strtol_base8() {
    let mut end: *mut u8 = core::ptr::null_mut();
    // SAFETY: Input is a valid C string; endptr is a valid out-pointer.
    unsafe {
        let v = strtol(b"017\0".as_ptr(), &mut end, 8);
        assert_eq!(v, 0o17); // 15 decimal
    }
}

fn test_strtol_base16() {
    let mut end: *mut u8 = core::ptr::null_mut();
    // SAFETY: Input is a valid C string; endptr is a valid out-pointer.
    unsafe {
        let v = strtol(b"0xff\0".as_ptr(), &mut end, 16);
        assert_eq!(v, 255);

        let v2 = strtol(b"1A\0".as_ptr(), &mut end, 16);
        assert_eq!(v2, 26);
    }
}

fn test_strtol_base0() {
    let mut end: *mut u8 = core::ptr::null_mut();
    // SAFETY: Input is a valid C string; endptr is a valid out-pointer.
    unsafe {
        // base 0: auto-detect from prefix
        let dec = strtol(b"42\0".as_ptr(), &mut end, 0);
        assert_eq!(dec, 42);

        let oct = strtol(b"010\0".as_ptr(), &mut end, 0);
        assert_eq!(oct, 8);

        let hex = strtol(b"0x10\0".as_ptr(), &mut end, 0);
        assert_eq!(hex, 16);
    }
}

fn test_strtoul() {
    let mut end: *mut u8 = core::ptr::null_mut();
    // SAFETY: Input is a valid C string; endptr is a valid out-pointer.
    unsafe {
        let v = strtoul(b"4294967295\0".as_ptr(), &mut end, 10);
        assert_eq!(v, u32::MAX as u64);

        let v2 = strtoul(b"0xff\0".as_ptr(), &mut end, 16);
        assert_eq!(v2, 255);

        let v3 = strtoul(b"0\0".as_ptr(), &mut end, 0);
        assert_eq!(v3, 0);
    }
}

fn test_strtod() {
    let mut end: *mut u8 = core::ptr::null_mut();
    // SAFETY: Input is a valid C string; endptr is a valid out-pointer.
    unsafe {
        let v = strtod(b"3.14\0".as_ptr(), &mut end);
        assert!((v - 3.14_f64).abs() < 1e-10);

        let neg = strtod(b"-2.5\0".as_ptr(), &mut end);
        assert!((neg - (-2.5_f64)).abs() < 1e-10);

        let zero = strtod(b"0.0\0".as_ptr(), &mut end);
        assert_eq!(zero, 0.0_f64);
    }
}

fn test_strtof() {
    let mut end: *mut u8 = core::ptr::null_mut();
    // SAFETY: Input is a valid C string; endptr is a valid out-pointer.
    unsafe {
        let v = strtof(b"1.5\0".as_ptr(), &mut end);
        assert!((v - 1.5_f32).abs() < 1e-5);

        let neg = strtof(b"-0.25\0".as_ptr(), &mut end);
        assert!((neg - (-0.25_f32)).abs() < 1e-5);
    }
}

fn test_strtoimax() {
    let mut end: *mut u8 = core::ptr::null_mut();
    // SAFETY: Input is a valid C string; endptr is a valid out-pointer.
    unsafe {
        let v = strtoimax(b"9223372036854775807\0".as_ptr(), &mut end, 10);
        assert_eq!(v, i64::MAX);

        let neg = strtoimax(b"-1\0".as_ptr(), &mut end, 10);
        assert_eq!(neg, -1i64);

        let hex = strtoimax(b"0x7f\0".as_ptr(), &mut end, 0);
        assert_eq!(hex, 127);
    }
}
