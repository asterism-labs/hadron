//! POSIX string and memory functions.
//!
//! All functions are `#[no_mangle] pub unsafe extern "C" fn` for C ABI export.
//! Implementations are straightforward byte-level operations suitable for
//! both host testing and cross-compilation.

/// Copy `n` bytes from `src` to `dest`. Regions must not overlap.
///
/// # Safety
///
/// `dest` and `src` must be valid for `n` bytes. Regions must not overlap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        // SAFETY: Caller guarantees valid, non-overlapping regions.
        unsafe { *dest.add(i) = *src.add(i) };
        i += 1;
    }
    dest
}

/// Copy `n` bytes from `src` to `dest`. Regions may overlap.
///
/// # Safety
///
/// `dest` and `src` must be valid for `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (dest as usize) < (src as usize) {
        // Copy forward.
        let mut i = 0;
        while i < n {
            // SAFETY: Caller guarantees valid pointers for n bytes.
            unsafe { *dest.add(i) = *src.add(i) };
            i += 1;
        }
    } else if (dest as usize) > (src as usize) {
        // Copy backward.
        let mut i = n;
        while i > 0 {
            i -= 1;
            // SAFETY: Caller guarantees valid pointers for n bytes.
            unsafe { *dest.add(i) = *src.add(i) };
        }
    }
    dest
}

/// Fill `n` bytes at `dest` with byte value `c`.
///
/// # Safety
///
/// `dest` must be valid for `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
    let byte = c as u8;
    let mut i = 0;
    while i < n {
        // SAFETY: Caller guarantees dest is valid for n bytes.
        unsafe { *dest.add(i) = byte };
        i += 1;
    }
    dest
}

/// Compare `n` bytes of `s1` and `s2`.
///
/// # Safety
///
/// `s1` and `s2` must be valid for `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        // SAFETY: Caller guarantees valid pointers for n bytes.
        let a = unsafe { *s1.add(i) };
        let b = unsafe { *s2.add(i) };
        if a != b {
            return (a as i32) - (b as i32);
        }
        i += 1;
    }
    0
}

/// Find first occurrence of byte `c` in first `n` bytes of `s`.
///
/// # Safety
///
/// `s` must be valid for `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memchr(s: *const u8, c: i32, n: usize) -> *const u8 {
    let byte = c as u8;
    let mut i = 0;
    while i < n {
        // SAFETY: Caller guarantees s is valid for n bytes.
        if unsafe { *s.add(i) } == byte {
            return unsafe { s.add(i) };
        }
        i += 1;
    }
    core::ptr::null()
}

/// Length of a NUL-terminated string.
///
/// # Safety
///
/// `s` must point to a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    let mut len = 0;
    // SAFETY: Caller guarantees NUL-terminated string.
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    len
}

/// Length of a NUL-terminated string, limited to `maxlen`.
///
/// # Safety
///
/// `s` must be valid for at least `maxlen` bytes or contain a NUL before that.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strnlen(s: *const u8, maxlen: usize) -> usize {
    let mut len = 0;
    // SAFETY: Caller guarantees valid region up to NUL or maxlen.
    while len < maxlen && unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    len
}

/// Compare two NUL-terminated strings.
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(s1: *const u8, s2: *const u8) -> i32 {
    let mut i = 0;
    loop {
        // SAFETY: Caller guarantees NUL-terminated strings.
        let a = unsafe { *s1.add(i) };
        let b = unsafe { *s2.add(i) };
        if a != b || a == 0 {
            return (a as i32) - (b as i32);
        }
        i += 1;
    }
}

/// Compare at most `n` bytes of two NUL-terminated strings.
///
/// # Safety
///
/// Both strings must be valid for at least `n` bytes or contain NUL before that.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        // SAFETY: Caller guarantees valid pointers.
        let a = unsafe { *s1.add(i) };
        let b = unsafe { *s2.add(i) };
        if a != b || a == 0 {
            return (a as i32) - (b as i32);
        }
        i += 1;
    }
    0
}

/// Find first occurrence of byte `c` in NUL-terminated string `s`.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strchr(s: *const u8, c: i32) -> *const u8 {
    let byte = c as u8;
    let mut i = 0;
    loop {
        // SAFETY: Caller guarantees NUL-terminated string.
        let ch = unsafe { *s.add(i) };
        if ch == byte {
            return unsafe { s.add(i) };
        }
        if ch == 0 {
            return core::ptr::null();
        }
        i += 1;
    }
}

/// Find last occurrence of byte `c` in NUL-terminated string `s`.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strrchr(s: *const u8, c: i32) -> *const u8 {
    let byte = c as u8;
    let mut last: *const u8 = core::ptr::null();
    let mut i = 0;
    loop {
        // SAFETY: Caller guarantees NUL-terminated string.
        let ch = unsafe { *s.add(i) };
        if ch == byte {
            last = unsafe { s.add(i) };
        }
        if ch == 0 {
            return last;
        }
        i += 1;
    }
}

/// Find first occurrence of `needle` in `haystack`.
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strstr(haystack: *const u8, needle: *const u8) -> *const u8 {
    // SAFETY: Caller guarantees NUL-terminated strings.
    let needle_len = unsafe { strlen(needle) };
    if needle_len == 0 {
        return haystack;
    }
    let haystack_len = unsafe { strlen(haystack) };
    if needle_len > haystack_len {
        return core::ptr::null();
    }
    let mut i = 0;
    while i <= haystack_len - needle_len {
        // SAFETY: Bounds checked above.
        if unsafe { memcmp(haystack.add(i), needle, needle_len) } == 0 {
            return unsafe { haystack.add(i) };
        }
        i += 1;
    }
    core::ptr::null()
}

/// Copy `src` to `dest` including NUL terminator.
///
/// # Safety
///
/// `dest` must have enough space. `src` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcpy(dest: *mut u8, src: *const u8) -> *mut u8 {
    let mut i = 0;
    loop {
        // SAFETY: Caller guarantees sufficient space and NUL-terminated src.
        let ch = unsafe { *src.add(i) };
        unsafe { *dest.add(i) = ch };
        if ch == 0 {
            return dest;
        }
        i += 1;
    }
}

/// Copy at most `n` bytes from `src` to `dest`, NUL-padding if shorter.
///
/// # Safety
///
/// `dest` must be valid for `n` bytes. `src` must be NUL-terminated or at least `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    // Copy src.
    while i < n {
        // SAFETY: Caller guarantees valid pointers.
        let ch = unsafe { *src.add(i) };
        unsafe { *dest.add(i) = ch };
        if ch == 0 {
            i += 1;
            break;
        }
        i += 1;
    }
    // NUL-pad remainder.
    while i < n {
        unsafe { *dest.add(i) = 0 };
        i += 1;
    }
    dest
}

/// Append `src` to end of `dest`.
///
/// # Safety
///
/// `dest` must have enough space. Both must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcat(dest: *mut u8, src: *const u8) -> *mut u8 {
    // SAFETY: Caller guarantees NUL-terminated dest with sufficient space.
    let dest_len = unsafe { strlen(dest) };
    unsafe { strcpy(dest.add(dest_len), src) };
    dest
}

/// Append at most `n` bytes from `src` to `dest`, plus NUL.
///
/// # Safety
///
/// `dest` must have enough space. Both must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncat(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: Caller guarantees NUL-terminated strings.
    let dest_len = unsafe { strlen(dest) };
    let mut i = 0;
    while i < n {
        let ch = unsafe { *src.add(i) };
        if ch == 0 {
            break;
        }
        unsafe { *dest.add(dest_len + i) = ch };
        i += 1;
    }
    unsafe { *dest.add(dest_len + i) = 0 };
    dest
}

/// Return a static error string for an errno value.
///
/// # Safety
///
/// Always safe to call (returns a static string pointer).
#[unsafe(no_mangle)]
pub extern "C" fn strerror(errnum: i32) -> *const u8 {
    let s: &[u8] = match errnum {
        0 => b"Success\0",
        1 => b"Operation not permitted\0",
        2 => b"No such file or directory\0",
        3 => b"No such process\0",
        4 => b"Interrupted system call\0",
        5 => b"I/O error\0",
        9 => b"Bad file descriptor\0",
        10 => b"No child processes\0",
        11 => b"Resource temporarily unavailable\0",
        12 => b"Cannot allocate memory\0",
        13 => b"Permission denied\0",
        14 => b"Bad address\0",
        17 => b"File exists\0",
        20 => b"Not a directory\0",
        21 => b"Is a directory\0",
        22 => b"Invalid argument\0",
        29 => b"Illegal seek\0",
        32 => b"Broken pipe\0",
        34 => b"Result too large\0",
        36 => b"File name too long\0",
        38 => b"Function not implemented\0",
        _ => b"Unknown error\0",
    };
    s.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memcpy() {
        let src = b"hello";
        let mut dest = [0u8; 5];
        unsafe { memcpy(dest.as_mut_ptr(), src.as_ptr(), 5) };
        assert_eq!(&dest, b"hello");
    }

    #[test]
    fn test_memmove_forward() {
        let mut buf = *b"abcdef";
        unsafe { memmove(buf.as_mut_ptr(), buf.as_ptr().add(2), 4) };
        assert_eq!(&buf[..4], b"cdef");
    }

    #[test]
    fn test_memmove_backward() {
        let mut buf = *b"abcdef";
        unsafe { memmove(buf.as_mut_ptr().add(2), buf.as_ptr(), 4) };
        assert_eq!(&buf[2..6], b"abcd");
    }

    #[test]
    fn test_memset() {
        let mut buf = [0u8; 4];
        unsafe { memset(buf.as_mut_ptr(), b'x' as i32, 4) };
        assert_eq!(&buf, b"xxxx");
    }

    #[test]
    fn test_memcmp_equal() {
        let a = b"abc";
        let b_arr = b"abc";
        assert_eq!(unsafe { memcmp(a.as_ptr(), b_arr.as_ptr(), 3) }, 0);
    }

    #[test]
    fn test_memcmp_less() {
        let a = b"abc";
        let b_arr = b"abd";
        assert!(unsafe { memcmp(a.as_ptr(), b_arr.as_ptr(), 3) } < 0);
    }

    #[test]
    fn test_memchr_found() {
        let buf = b"hello";
        let p = unsafe { memchr(buf.as_ptr(), b'l' as i32, 5) };
        assert_eq!(p, unsafe { buf.as_ptr().add(2) });
    }

    #[test]
    fn test_memchr_not_found() {
        let buf = b"hello";
        let p = unsafe { memchr(buf.as_ptr(), b'z' as i32, 5) };
        assert!(p.is_null());
    }

    #[test]
    fn test_strlen_basic() {
        let s = b"hello\0";
        assert_eq!(unsafe { strlen(s.as_ptr()) }, 5);
    }

    #[test]
    fn test_strlen_empty() {
        let s = b"\0";
        assert_eq!(unsafe { strlen(s.as_ptr()) }, 0);
    }

    #[test]
    fn test_strnlen_limited() {
        let s = b"hello\0";
        assert_eq!(unsafe { strnlen(s.as_ptr(), 3) }, 3);
    }

    #[test]
    fn test_strnlen_nul_before_max() {
        let s = b"hi\0";
        assert_eq!(unsafe { strnlen(s.as_ptr(), 10) }, 2);
    }

    #[test]
    fn test_strcmp_equal() {
        let a = b"abc\0";
        let b_arr = b"abc\0";
        assert_eq!(unsafe { strcmp(a.as_ptr(), b_arr.as_ptr()) }, 0);
    }

    #[test]
    fn test_strcmp_less() {
        let a = b"abc\0";
        let b_arr = b"abd\0";
        assert!(unsafe { strcmp(a.as_ptr(), b_arr.as_ptr()) } < 0);
    }

    #[test]
    fn test_strcmp_greater() {
        let a = b"abd\0";
        let b_arr = b"abc\0";
        assert!(unsafe { strcmp(a.as_ptr(), b_arr.as_ptr()) } > 0);
    }

    #[test]
    fn test_strncmp_limited() {
        let a = b"abcX\0";
        let b_arr = b"abcY\0";
        assert_eq!(unsafe { strncmp(a.as_ptr(), b_arr.as_ptr(), 3) }, 0);
    }

    #[test]
    fn test_strchr_found() {
        let s = b"hello\0";
        let p = unsafe { strchr(s.as_ptr(), b'l' as i32) };
        assert_eq!(p, unsafe { s.as_ptr().add(2) });
    }

    #[test]
    fn test_strchr_nul() {
        let s = b"hi\0";
        let p = unsafe { strchr(s.as_ptr(), 0) };
        assert_eq!(p, unsafe { s.as_ptr().add(2) });
    }

    #[test]
    fn test_strrchr() {
        let s = b"hello\0";
        let p = unsafe { strrchr(s.as_ptr(), b'l' as i32) };
        assert_eq!(p, unsafe { s.as_ptr().add(3) });
    }

    #[test]
    fn test_strstr_found() {
        let h = b"hello world\0";
        let n = b"world\0";
        let p = unsafe { strstr(h.as_ptr(), n.as_ptr()) };
        assert_eq!(p, unsafe { h.as_ptr().add(6) });
    }

    #[test]
    fn test_strstr_empty_needle() {
        let h = b"hello\0";
        let n = b"\0";
        let p = unsafe { strstr(h.as_ptr(), n.as_ptr()) };
        assert_eq!(p, h.as_ptr());
    }

    #[test]
    fn test_strstr_not_found() {
        let h = b"hello\0";
        let n = b"xyz\0";
        let p = unsafe { strstr(h.as_ptr(), n.as_ptr()) };
        assert!(p.is_null());
    }

    #[test]
    fn test_strcpy() {
        let src = b"hello\0";
        let mut dest = [0u8; 6];
        unsafe { strcpy(dest.as_mut_ptr(), src.as_ptr()) };
        assert_eq!(&dest, b"hello\0");
    }

    #[test]
    fn test_strncpy_pad() {
        let src = b"hi\0";
        let mut dest = [b'x'; 6];
        unsafe { strncpy(dest.as_mut_ptr(), src.as_ptr(), 6) };
        assert_eq!(&dest, b"hi\0\0\0\0");
    }

    #[test]
    fn test_strcat() {
        let mut buf = [0u8; 12];
        buf[..6].copy_from_slice(b"hello\0");
        let src = b" world\0";
        unsafe { strcat(buf.as_mut_ptr(), src.as_ptr()) };
        assert_eq!(&buf[..12], b"hello world\0");
    }

    #[test]
    fn test_strncat() {
        let mut buf = [0u8; 10];
        buf[..3].copy_from_slice(b"hi\0");
        let src = b"world\0";
        unsafe { strncat(buf.as_mut_ptr(), src.as_ptr(), 3) };
        assert_eq!(&buf[..6], b"hiwor\0");
    }

    #[test]
    fn test_strerror_known() {
        let p = strerror(2);
        let s = unsafe { core::ffi::CStr::from_ptr(p.cast()) };
        assert_eq!(s.to_str().unwrap(), "No such file or directory");
    }

    #[test]
    fn test_strerror_unknown() {
        let p = strerror(999);
        let s = unsafe { core::ffi::CStr::from_ptr(p.cast()) };
        assert_eq!(s.to_str().unwrap(), "Unknown error");
    }
}
