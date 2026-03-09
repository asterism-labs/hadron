//! utest: POSIX string function regression tests.
//!
//! Covers: `strlen`, `strcmp`, `strncmp`, `strcpy`, `strncpy`, `strcat`,
//! `strncat`, `strchr`, `strrchr`, `strstr`, `memset`, `memcpy`, `memmove`,
//! `memcmp`, `strtok_r`.

#![no_std]
#![no_main]

// Force hadron_libc_core to be linked so its #[no_mangle] symbols (strlen,
// strcpy, …) are available to the `extern "C"` declarations below.
extern crate hadron_libc_core;

use hadron_utest::utest_main;

utest_main!(
    test_strlen,
    test_strcmp,
    test_strncmp,
    test_strcpy,
    test_strncpy,
    test_strcat,
    test_strncat,
    test_strchr,
    test_strrchr,
    test_strstr,
    test_memset,
    test_memcpy,
    test_memmove,
    test_memcmp,
    test_strtok_r,
);

// ── extern declarations ───────────────────────────────────────────────────────

unsafe extern "C" {
    fn strlen(s: *const u8) -> usize;
    fn strcmp(a: *const u8, b: *const u8) -> i32;
    fn strncmp(a: *const u8, b: *const u8, n: usize) -> i32;
    fn strcpy(dst: *mut u8, src: *const u8) -> *mut u8;
    fn strncpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8;
    fn strcat(dst: *mut u8, src: *const u8) -> *mut u8;
    fn strncat(dst: *mut u8, src: *const u8, n: usize) -> *mut u8;
    fn strchr(s: *const u8, c: i32) -> *mut u8;
    fn strrchr(s: *const u8, c: i32) -> *mut u8;
    fn strstr(haystack: *const u8, needle: *const u8) -> *mut u8;
    fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8;
    fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8;
    fn memmove(dst: *mut u8, src: *const u8, n: usize) -> *mut u8;
    fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32;
    fn strtok_r(s: *mut u8, delim: *const u8, saveptr: *mut *mut u8) -> *mut u8;
}

// ── tests ─────────────────────────────────────────────────────────────────────

fn test_strlen() {
    // SAFETY: All pointers are valid C strings.
    unsafe {
        assert_eq!(strlen(b"hello\0".as_ptr()), 5);
        assert_eq!(strlen(b"\0".as_ptr()), 0);
        assert_eq!(strlen(b"abc\0".as_ptr()), 3);
    }
}

fn test_strcmp() {
    // SAFETY: All pointers are valid C strings.
    unsafe {
        assert_eq!(strcmp(b"abc\0".as_ptr(), b"abc\0".as_ptr()), 0);
        assert!(strcmp(b"abc\0".as_ptr(), b"abd\0".as_ptr()) < 0);
        assert!(strcmp(b"abd\0".as_ptr(), b"abc\0".as_ptr()) > 0);
        assert!(strcmp(b"abc\0".as_ptr(), b"\0".as_ptr()) > 0);
        assert!(strcmp(b"\0".as_ptr(), b"abc\0".as_ptr()) < 0);
    }
}

fn test_strncmp() {
    // SAFETY: All pointers are valid C strings.
    unsafe {
        assert_eq!(strncmp(b"abcX\0".as_ptr(), b"abcY\0".as_ptr(), 3), 0);
        assert!(strncmp(b"abcX\0".as_ptr(), b"abcY\0".as_ptr(), 4) < 0);
        assert_eq!(strncmp(b"abc\0".as_ptr(), b"abc\0".as_ptr(), 100), 0);
        assert_eq!(strncmp(b"abc\0".as_ptr(), b"abc\0".as_ptr(), 0), 0);
    }
}

fn test_strcpy() {
    let mut buf = [0u8; 16];
    // SAFETY: buf has enough space; src is a valid C string shorter than buf.
    unsafe {
        strcpy(buf.as_mut_ptr(), b"hello\0".as_ptr());
        assert_eq!(&buf[..6], b"hello\0");
    }
}

fn test_strncpy() {
    let mut buf = [0xffu8; 8];
    // SAFETY: buf has 8 bytes, n=7, src is a valid C string.
    unsafe {
        strncpy(buf.as_mut_ptr(), b"hi\0".as_ptr(), 7);
        // strncpy pads with NUL up to n
        assert_eq!(buf[0], b'h');
        assert_eq!(buf[1], b'i');
        assert_eq!(buf[2], 0);
        assert_eq!(buf[3], 0);
    }
}

fn test_strcat() {
    let mut buf = [0u8; 16];
    // SAFETY: buf large enough; both strings are valid C strings.
    unsafe {
        strcpy(buf.as_mut_ptr(), b"foo\0".as_ptr());
        strcat(buf.as_mut_ptr(), b"bar\0".as_ptr());
        assert_eq!(strlen(buf.as_ptr()), 6);
        assert_eq!(&buf[..7], b"foobar\0");
    }
}

fn test_strncat() {
    let mut buf = [0u8; 16];
    // SAFETY: buf large enough; all pointers are valid C strings.
    unsafe {
        strcpy(buf.as_mut_ptr(), b"foo\0".as_ptr());
        strncat(buf.as_mut_ptr(), b"barbaz\0".as_ptr(), 3);
        assert_eq!(strlen(buf.as_ptr()), 6);
        assert_eq!(&buf[..7], b"foobar\0");
    }
}

fn test_strchr() {
    // SAFETY: All pointers are valid C strings.
    unsafe {
        let s = b"abcabc\0";
        let p = strchr(s.as_ptr(), b'b' as i32);
        assert!(!p.is_null());
        // SAFETY: p is within s.
        assert_eq!(*p, b'b');
        // first occurrence at index 1
        assert_eq!(p as usize - s.as_ptr() as usize, 1);

        // search for NUL terminator
        let end = strchr(s.as_ptr(), 0);
        assert!(!end.is_null());
        assert_eq!(end as usize - s.as_ptr() as usize, 6);

        // absent character
        let q = strchr(s.as_ptr(), b'z' as i32);
        assert!(q.is_null());
    }
}

fn test_strrchr() {
    // SAFETY: All pointers are valid C strings.
    unsafe {
        let s = b"abcabc\0";
        let p = strrchr(s.as_ptr(), b'b' as i32);
        assert!(!p.is_null());
        // last occurrence at index 4
        assert_eq!(p as usize - s.as_ptr() as usize, 4);

        let q = strrchr(s.as_ptr(), b'z' as i32);
        assert!(q.is_null());
    }
}

fn test_strstr() {
    // SAFETY: All pointers are valid C strings.
    unsafe {
        let hay = b"hello world\0";
        let p = strstr(hay.as_ptr(), b"world\0".as_ptr());
        assert!(!p.is_null());
        assert_eq!(p as usize - hay.as_ptr() as usize, 6);

        let q = strstr(hay.as_ptr(), b"xyz\0".as_ptr());
        assert!(q.is_null());

        // empty needle returns haystack
        let r = strstr(hay.as_ptr(), b"\0".as_ptr());
        assert_eq!(r as usize, hay.as_ptr() as usize);
    }
}

fn test_memset() {
    let mut buf = [0u8; 8];
    // SAFETY: buf is valid and 8 bytes long.
    unsafe {
        memset(buf.as_mut_ptr(), 0xABu8 as i32, 4);
    }
    assert_eq!(&buf[..4], &[0xAB, 0xAB, 0xAB, 0xAB]);
    assert_eq!(&buf[4..], &[0, 0, 0, 0]);
}

fn test_memcpy() {
    let src = [1u8, 2, 3, 4, 5];
    let mut dst = [0u8; 5];
    // SAFETY: src and dst are non-overlapping valid slices of 5 bytes.
    unsafe {
        memcpy(dst.as_mut_ptr(), src.as_ptr(), 5);
    }
    assert_eq!(dst, src);
}

fn test_memmove() {
    // Test overlapping move (shift right by 2 within the same buffer).
    let mut buf = [1u8, 2, 3, 4, 5, 0, 0];
    // SAFETY: src and dst overlap; memmove handles this correctly.
    unsafe {
        memmove(buf.as_mut_ptr().add(2), buf.as_ptr(), 5);
    }
    assert_eq!(&buf[2..7], &[1, 2, 3, 4, 5]);

    // Non-overlapping: shift left
    let mut buf2 = [0u8, 1, 2, 3, 4, 0, 0];
    // SAFETY: src offset 1 and dst offset 0 overlap for 5 bytes.
    unsafe {
        memmove(buf2.as_mut_ptr(), buf2.as_ptr().add(1), 5);
    }
    assert_eq!(&buf2[..5], &[1, 2, 3, 4, 0]);
}

fn test_memcmp() {
    // SAFETY: All slices are valid byte arrays.
    unsafe {
        assert_eq!(memcmp(b"abc".as_ptr(), b"abc".as_ptr(), 3), 0);
        assert!(memcmp(b"abc".as_ptr(), b"abd".as_ptr(), 3) < 0);
        assert!(memcmp(b"abd".as_ptr(), b"abc".as_ptr(), 3) > 0);
        assert_eq!(memcmp(b"abc".as_ptr(), b"abd".as_ptr(), 2), 0);
        assert_eq!(memcmp(b"abc".as_ptr(), b"xyz".as_ptr(), 0), 0);
    }
}

fn test_strtok_r() {
    let mut s = *b"one:two::three\0";
    let mut saveptr: *mut u8 = core::ptr::null_mut();
    let delim = b":\0";

    // SAFETY: s is a mutable NUL-terminated buffer; delim is a valid C string.
    unsafe {
        let t1 = strtok_r(s.as_mut_ptr(), delim.as_ptr(), &mut saveptr);
        assert!(!t1.is_null());
        assert_eq!(strlen(t1), 3); // "one"

        let t2 = strtok_r(core::ptr::null_mut(), delim.as_ptr(), &mut saveptr);
        assert!(!t2.is_null());
        assert_eq!(strlen(t2), 3); // "two"

        let t3 = strtok_r(core::ptr::null_mut(), delim.as_ptr(), &mut saveptr);
        assert!(!t3.is_null());
        assert_eq!(strlen(t3), 5); // "three"

        let t4 = strtok_r(core::ptr::null_mut(), delim.as_ptr(), &mut saveptr);
        assert!(t4.is_null()); // exhausted
    }
}
