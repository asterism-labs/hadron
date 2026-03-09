//! utest: POSIX character classification and conversion regression tests.
//!
//! Covers: `isalpha`, `isdigit`, `isalnum`, `isspace`, `isupper`, `islower`,
//! `isxdigit`, `toupper`, `tolower`.

#![no_std]
#![no_main]

// Force hadron_libc_core to be linked so its #[no_mangle] symbols (isalpha,
// toupper, …) are available to the `extern "C"` declarations below.
extern crate hadron_libc_core;

use hadron_utest::utest_main;

utest_main!(
    test_isalpha,
    test_isdigit,
    test_isalnum,
    test_isspace,
    test_isupper,
    test_islower,
    test_isxdigit,
    test_toupper,
    test_tolower,
);

// ── extern declarations ───────────────────────────────────────────────────────

unsafe extern "C" {
    fn isalpha(c: i32) -> i32;
    fn isdigit(c: i32) -> i32;
    fn isalnum(c: i32) -> i32;
    fn isspace(c: i32) -> i32;
    fn isupper(c: i32) -> i32;
    fn islower(c: i32) -> i32;
    fn isxdigit(c: i32) -> i32;
    fn toupper(c: i32) -> i32;
    fn tolower(c: i32) -> i32;
}

// ── tests ─────────────────────────────────────────────────────────────────────

fn test_isalpha() {
    // SAFETY: ctype functions take plain ints; no pointer safety concern.
    unsafe {
        assert_ne!(isalpha(b'a' as i32), 0);
        assert_ne!(isalpha(b'Z' as i32), 0);
        assert_eq!(isalpha(b'0' as i32), 0);
        assert_eq!(isalpha(b'!' as i32), 0);
        assert_eq!(isalpha(b' ' as i32), 0);
    }
}

fn test_isdigit() {
    // SAFETY: ctype functions take plain ints.
    unsafe {
        assert_ne!(isdigit(b'0' as i32), 0);
        assert_ne!(isdigit(b'9' as i32), 0);
        assert_eq!(isdigit(b'a' as i32), 0);
        assert_eq!(isdigit(b' ' as i32), 0);
    }
}

fn test_isalnum() {
    // SAFETY: ctype functions take plain ints.
    unsafe {
        assert_ne!(isalnum(b'a' as i32), 0);
        assert_ne!(isalnum(b'Z' as i32), 0);
        assert_ne!(isalnum(b'5' as i32), 0);
        assert_eq!(isalnum(b'!' as i32), 0);
        assert_eq!(isalnum(b' ' as i32), 0);
    }
}

fn test_isspace() {
    // SAFETY: ctype functions take plain ints.
    unsafe {
        assert_ne!(isspace(b' ' as i32), 0);
        assert_ne!(isspace(b'\t' as i32), 0);
        assert_ne!(isspace(b'\n' as i32), 0);
        assert_ne!(isspace(b'\r' as i32), 0);
        assert_ne!(isspace(0x0C), 0); // form feed
        assert_ne!(isspace(0x0B), 0); // vertical tab
        assert_eq!(isspace(b'a' as i32), 0);
        assert_eq!(isspace(b'0' as i32), 0);
    }
}

fn test_isupper() {
    // SAFETY: ctype functions take plain ints.
    unsafe {
        assert_ne!(isupper(b'A' as i32), 0);
        assert_ne!(isupper(b'Z' as i32), 0);
        assert_eq!(isupper(b'a' as i32), 0);
        assert_eq!(isupper(b'0' as i32), 0);
    }
}

fn test_islower() {
    // SAFETY: ctype functions take plain ints.
    unsafe {
        assert_ne!(islower(b'a' as i32), 0);
        assert_ne!(islower(b'z' as i32), 0);
        assert_eq!(islower(b'A' as i32), 0);
        assert_eq!(islower(b'0' as i32), 0);
    }
}

fn test_isxdigit() {
    // SAFETY: ctype functions take plain ints.
    unsafe {
        assert_ne!(isxdigit(b'0' as i32), 0);
        assert_ne!(isxdigit(b'9' as i32), 0);
        assert_ne!(isxdigit(b'a' as i32), 0);
        assert_ne!(isxdigit(b'f' as i32), 0);
        assert_ne!(isxdigit(b'A' as i32), 0);
        assert_ne!(isxdigit(b'F' as i32), 0);
        assert_eq!(isxdigit(b'g' as i32), 0);
        assert_eq!(isxdigit(b'z' as i32), 0);
        assert_eq!(isxdigit(b' ' as i32), 0);
    }
}

fn test_toupper() {
    // SAFETY: ctype functions take plain ints.
    unsafe {
        assert_eq!(toupper(b'a' as i32), b'A' as i32);
        assert_eq!(toupper(b'z' as i32), b'Z' as i32);
        assert_eq!(toupper(b'A' as i32), b'A' as i32); // already upper
        assert_eq!(toupper(b'0' as i32), b'0' as i32); // not a letter
    }
}

fn test_tolower() {
    // SAFETY: ctype functions take plain ints.
    unsafe {
        assert_eq!(tolower(b'A' as i32), b'a' as i32);
        assert_eq!(tolower(b'Z' as i32), b'z' as i32);
        assert_eq!(tolower(b'a' as i32), b'a' as i32); // already lower
        assert_eq!(tolower(b'0' as i32), b'0' as i32); // not a letter
    }
}
