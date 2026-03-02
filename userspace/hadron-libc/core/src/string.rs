//! POSIX string and memory functions.
//!
//! Covers C89/C99 core (`memcpy`, `strlen`, `strcmp`, …), POSIX.1-2001
//! extensions (`strtok_r`, `strerror_r`), and BSD extensions (`strcasecmp`,
//! `strdup`/`strndup`).
//!
//! Pure string operations (no I/O, no allocation) are always available and
//! host-testable. Functions that require heap allocation (`strdup`, `strndup`)
//! are gated behind `#[cfg(feature = "userspace")]`.

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

/// Length of prefix of `s` consisting entirely of bytes in `accept`.
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strspn(s: *const u8, accept: *const u8) -> usize {
    let mut count = 0;
    loop {
        // SAFETY: Caller guarantees NUL-terminated strings.
        let ch = unsafe { *s.add(count) };
        if ch == 0 {
            break;
        }
        let mut found = false;
        let mut j = 0;
        loop {
            // SAFETY: accept is NUL-terminated.
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

/// Length of prefix of `s` containing no byte from `reject`.
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcspn(s: *const u8, reject: *const u8) -> usize {
    let mut count = 0;
    loop {
        // SAFETY: Caller guarantees NUL-terminated strings.
        let ch = unsafe { *s.add(count) };
        if ch == 0 {
            break;
        }
        let mut j = 0;
        loop {
            // SAFETY: reject is NUL-terminated.
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

/// Pointer to first byte of `s` that is in `accept`, or null.
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strpbrk(s: *const u8, accept: *const u8) -> *const u8 {
    // SAFETY: Caller guarantees NUL-terminated strings.
    let pos = unsafe { strcspn(s, accept) };
    if unsafe { *s.add(pos) } == 0 {
        core::ptr::null()
    } else {
        // SAFETY: pos is within the string.
        unsafe { s.add(pos) }
    }
}

/// Case-insensitive comparison of two NUL-terminated strings.
///
/// Returns negative, zero, or positive as `s1` is less than, equal to,
/// or greater than `s2` (ignoring case).
///
/// # Safety
///
/// Both strings must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcasecmp(s1: *const u8, s2: *const u8) -> i32 {
    let mut i = 0;
    loop {
        // SAFETY: Caller guarantees NUL-terminated strings.
        let a = unsafe { *s1.add(i) }.to_ascii_lowercase();
        let b = unsafe { *s2.add(i) }.to_ascii_lowercase();
        if a != b || a == 0 {
            return (a as i32) - (b as i32);
        }
        i += 1;
    }
}

/// Case-insensitive comparison of at most `n` bytes.
///
/// # Safety
///
/// Both strings must be valid for `n` bytes or NUL-terminated sooner.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncasecmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        // SAFETY: Caller guarantees valid range.
        let a = unsafe { *s1.add(i) }.to_ascii_lowercase();
        let b = unsafe { *s2.add(i) }.to_ascii_lowercase();
        if a != b || a == 0 {
            return (a as i32) - (b as i32);
        }
        i += 1;
    }
    0
}

/// Reentrant string tokenizer.
///
/// On first call, `str` is the string to tokenize. On subsequent calls,
/// pass `null` and `*saveptr` retains state between calls.
///
/// # Safety
///
/// `str` (or `*saveptr`) must be a valid mutable NUL-terminated string.
/// `delim` must be a valid NUL-terminated string. `saveptr` must be non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok_r(
    str: *mut u8,
    delim: *const u8,
    saveptr: *mut *mut u8,
) -> *mut u8 {
    let mut s = if str.is_null() {
        // SAFETY: saveptr is non-null per contract.
        unsafe { *saveptr }
    } else {
        str
    };
    if s.is_null() {
        return core::ptr::null_mut();
    }
    // Skip leading delimiters.
    // SAFETY: s is a valid NUL-terminated string.
    s = unsafe { s.add(strspn(s, delim)) };
    if unsafe { *s } == 0 {
        // SAFETY: saveptr is non-null.
        unsafe { *saveptr = core::ptr::null_mut() };
        return core::ptr::null_mut();
    }
    let token = s;
    // Find end of token.
    // SAFETY: token is within the original string.
    s = unsafe { s.add(strcspn(s, delim)) };
    if unsafe { *s } != 0 {
        // SAFETY: s points within the original mutable string.
        unsafe { *s = 0 };
        unsafe { *saveptr = s.add(1) };
    } else {
        // SAFETY: saveptr is non-null.
        unsafe { *saveptr = core::ptr::null_mut() };
    }
    token
}

/// Thread-local state for the non-reentrant `strtok`.
// Phase 2: single static (non-thread-safe). Replace with TLS in Phase 5.
#[allow(dead_code)] // Phase 5: replace with TLS
static mut STRTOK_SAVE: *mut u8 = core::ptr::null_mut();

/// Non-reentrant string tokenizer. Prefer `strtok_r` in new code.
///
/// # Safety
///
/// `str` must be a valid mutable NUL-terminated string on first call.
/// `delim` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok(str: *mut u8, delim: *const u8) -> *mut u8 {
    // SAFETY: STRTOK_SAVE is valid to pass to strtok_r; single-threaded phase.
    unsafe { strtok_r(str, delim, core::ptr::addr_of_mut!(STRTOK_SAVE)) }
}

/// Write a description of `errnum` into `buf[..buflen]`.
///
/// Returns `0` on success or an error code if `buf` is too small or `errnum`
/// is unknown. The output is always NUL-terminated when `buflen > 0`.
///
/// # Safety
///
/// `buf` must be valid for `buflen` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strerror_r(errnum: i32, buf: *mut u8, buflen: usize) -> i32 {
    // SAFETY: Caller guarantees buf is valid for buflen bytes.
    let src = strerror(errnum);
    // SAFETY: src is a NUL-terminated static string.
    let src_len = unsafe { strlen(src) };
    if buflen == 0 {
        return crate::errno::ERANGE.0;
    }
    let copy_len = if src_len < buflen {
        src_len
    } else {
        buflen - 1
    };
    // SAFETY: buf is valid for buflen bytes; src is valid for src_len+1 bytes.
    unsafe { core::ptr::copy_nonoverlapping(src, buf, copy_len) };
    unsafe { *buf.add(copy_len) = 0 };
    if src_len >= buflen {
        crate::errno::ERANGE.0
    } else {
        0
    }
}

/// Return the name of signal `sig` as a static string.
///
/// Returns `"Unknown signal"` for unrecognized signal numbers.
///
/// # Safety
///
/// Always safe to call.
#[unsafe(no_mangle)]
pub extern "C" fn strsignal(sig: i32) -> *const u8 {
    let s: &[u8] = match sig {
        1 => b"Hangup\0",
        2 => b"Interrupt\0",
        3 => b"Quit\0",
        4 => b"Illegal instruction\0",
        5 => b"Trace/breakpoint trap\0",
        6 => b"Aborted\0",
        7 => b"Bus error\0",
        8 => b"Floating point exception\0",
        9 => b"Killed\0",
        10 => b"User defined signal 1\0",
        11 => b"Segmentation fault\0",
        12 => b"User defined signal 2\0",
        13 => b"Broken pipe\0",
        14 => b"Alarm clock\0",
        15 => b"Terminated\0",
        17 => b"Child exited\0",
        18 => b"Continued\0",
        19 => b"Stopped (signal)\0",
        20 => b"Stopped\0",
        _ => b"Unknown signal\0",
    };
    s.as_ptr()
}

/// Find first byte of `s` that is not `c`, or the NUL terminator.
///
/// GNU extension: always returns a non-null pointer.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strchrnul(s: *const u8, c: i32) -> *const u8 {
    let byte = c as u8;
    let mut i = 0;
    loop {
        // SAFETY: Caller guarantees NUL-terminated string.
        let ch = unsafe { *s.add(i) };
        if ch == byte || ch == 0 {
            return unsafe { s.add(i) };
        }
        i += 1;
    }
}

/// Copy `n` bytes from `src` to `dest` and return a pointer past the last byte written.
///
/// GNU extension: like `memcpy` but returns `dest + n` instead of `dest`.
///
/// # Safety
///
/// `dest` and `src` must be valid for `n` bytes. Regions must not overlap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mempcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: Caller guarantees non-overlapping regions valid for n bytes.
    unsafe { memcpy(dest, src, n) };
    // SAFETY: dest is valid for n bytes.
    unsafe { dest.add(n) }
}

/// BSD-style safe `strcpy`: copy at most `size-1` bytes, always NUL-terminate.
///
/// Returns the length of `src` (not truncated), allowing truncation detection.
///
/// # Safety
///
/// `dest` must be valid for `size` bytes. `src` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlcpy(dest: *mut u8, src: *const u8, size: usize) -> usize {
    // SAFETY: Caller guarantees NUL-terminated src.
    let src_len = unsafe { strlen(src) };
    if size == 0 {
        return src_len;
    }
    let copy_len = if src_len < size { src_len } else { size - 1 };
    // SAFETY: dest is valid for size bytes; src is valid for src_len bytes.
    unsafe { core::ptr::copy_nonoverlapping(src, dest, copy_len) };
    unsafe { *dest.add(copy_len) = 0 };
    src_len
}

/// BSD-style safe `strcat`: append at most `size - strlen(dest) - 1` bytes.
///
/// Returns `strlen(src) + strlen(dest)` (before truncation), for detection.
///
/// # Safety
///
/// `dest` must be valid for `size` bytes and NUL-terminated. `src` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlcat(dest: *mut u8, src: *const u8, size: usize) -> usize {
    // SAFETY: Caller guarantees NUL-terminated strings.
    let dest_len = unsafe { strnlen(dest, size) };
    let src_len = unsafe { strlen(src) };
    if dest_len == size {
        return size + src_len;
    }
    let available = size - dest_len - 1;
    let copy_len = if src_len <= available {
        src_len
    } else {
        available
    };
    // SAFETY: dest+dest_len is within dest's valid region; src is valid for src_len.
    unsafe { core::ptr::copy_nonoverlapping(src, dest.add(dest_len), copy_len) };
    unsafe { *dest.add(dest_len + copy_len) = 0 };
    dest_len + src_len
}

// ---- strdup / strndup (require heap allocation) ----------------------------

#[cfg(feature = "userspace")]
use crate::alloc::malloc;

/// Allocate a copy of NUL-terminated string `s`.
///
/// Returns null if `s` is null or allocation fails.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[cfg(feature = "userspace")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strdup(s: *const u8) -> *mut u8 {
    if s.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: Caller guarantees NUL-terminated string.
    let len = unsafe { strlen(s) };
    // SAFETY: malloc returns valid memory or null.
    let p = unsafe { malloc(len + 1) };
    if p.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: p is valid for len+1 bytes; s is valid for len+1 bytes.
    unsafe { core::ptr::copy_nonoverlapping(s, p, len + 1) };
    p
}

/// Allocate a copy of at most `n` bytes of string `s`.
///
/// The result is always NUL-terminated. Returns null on allocation failure.
///
/// # Safety
///
/// `s` must be a valid string of at least `n` bytes or NUL-terminated sooner.
#[cfg(feature = "userspace")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strndup(s: *const u8, n: usize) -> *mut u8 {
    if s.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: Caller guarantees valid region up to NUL or n bytes.
    let len = unsafe { strnlen(s, n) };
    // SAFETY: malloc returns valid memory or null.
    let p = unsafe { malloc(len + 1) };
    if p.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: p is valid for len+1 bytes; s is valid for len bytes.
    unsafe { core::ptr::copy_nonoverlapping(s, p, len) };
    unsafe { *p.add(len) = 0 };
    p
}

/// Apply locale-specific transformation to `src`, writing at most `n` bytes to `dest`.
///
/// For Hadron: locale is always C/POSIX, so this is equivalent to `strncpy`.
///
/// # Safety
///
/// `dest` must be valid for `n` bytes. `src` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strxfrm(dest: *mut u8, src: *const u8, n: usize) -> usize {
    // SAFETY: Caller guarantees valid pointers.
    let src_len = unsafe { strlen(src) };
    if n > 0 {
        let copy_n = if src_len < n { src_len + 1 } else { n };
        unsafe { core::ptr::copy_nonoverlapping(src, dest, copy_n) };
    }
    src_len
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

    // ---- strspn / strcspn / strpbrk -----------------------------------------

    #[test]
    fn test_strspn_all_accept() {
        let s = b"aaabbb\0";
        let accept = b"ab\0";
        assert_eq!(unsafe { strspn(s.as_ptr(), accept.as_ptr()) }, 6);
    }

    #[test]
    fn test_strspn_none() {
        let s = b"xyz\0";
        let accept = b"abc\0";
        assert_eq!(unsafe { strspn(s.as_ptr(), accept.as_ptr()) }, 0);
    }

    #[test]
    fn test_strcspn_stops_at_reject() {
        let s = b"hello world\0";
        let reject = b" \0";
        assert_eq!(unsafe { strcspn(s.as_ptr(), reject.as_ptr()) }, 5);
    }

    #[test]
    fn test_strpbrk_found() {
        let s = b"hello\0";
        let accept = b"lo\0";
        let p = unsafe { strpbrk(s.as_ptr(), accept.as_ptr()) };
        assert_eq!(p, unsafe { s.as_ptr().add(2) }); // 'l' at index 2
    }

    #[test]
    fn test_strpbrk_not_found() {
        let s = b"hello\0";
        let accept = b"xyz\0";
        let p = unsafe { strpbrk(s.as_ptr(), accept.as_ptr()) };
        assert!(p.is_null());
    }

    // ---- strcasecmp / strncasecmp -------------------------------------------

    #[test]
    fn test_strcasecmp_equal() {
        let a = b"Hello\0";
        let b = b"hello\0";
        assert_eq!(unsafe { strcasecmp(a.as_ptr(), b.as_ptr()) }, 0);
    }

    #[test]
    fn test_strcasecmp_different() {
        let a = b"ABC\0";
        let b = b"ABD\0";
        assert!(unsafe { strcasecmp(a.as_ptr(), b.as_ptr()) } < 0);
    }

    #[test]
    fn test_strncasecmp_partial() {
        let a = b"HelloWorld\0";
        let b = b"HELLOXXX\0";
        assert_eq!(unsafe { strncasecmp(a.as_ptr(), b.as_ptr(), 5) }, 0);
    }

    // ---- strtok_r -----------------------------------------------------------

    #[test]
    fn test_strtok_r_basic() {
        let mut input = *b"one two three\0";
        let delim = b" \0";
        let mut save: *mut u8 = core::ptr::null_mut();

        let t1 = unsafe { strtok_r(input.as_mut_ptr(), delim.as_ptr(), &mut save) };
        assert_eq!(unsafe { strlen(t1) }, 3);

        let t2 = unsafe { strtok_r(core::ptr::null_mut(), delim.as_ptr(), &mut save) };
        assert_eq!(unsafe { strlen(t2) }, 3);

        let t3 = unsafe { strtok_r(core::ptr::null_mut(), delim.as_ptr(), &mut save) };
        assert_eq!(unsafe { strlen(t3) }, 5);

        let t4 = unsafe { strtok_r(core::ptr::null_mut(), delim.as_ptr(), &mut save) };
        assert!(t4.is_null());
    }

    // ---- strerror_r ---------------------------------------------------------

    #[test]
    fn test_strerror_r_success() {
        let mut buf = [0u8; 64];
        let ret = unsafe { strerror_r(2, buf.as_mut_ptr(), buf.len()) };
        assert_eq!(ret, 0);
        let s = core::ffi::CStr::from_bytes_until_nul(&buf).unwrap();
        assert_eq!(s.to_str().unwrap(), "No such file or directory");
    }

    #[test]
    fn test_strerror_r_truncated() {
        let mut buf = [0u8; 4];
        let ret = unsafe { strerror_r(2, buf.as_mut_ptr(), buf.len()) };
        assert_ne!(ret, 0); // ERANGE
        assert_eq!(buf[3], 0); // still NUL-terminated
    }

    // ---- strsignal ----------------------------------------------------------

    #[test]
    fn test_strsignal_sigkill() {
        let p = strsignal(9);
        let s = unsafe { core::ffi::CStr::from_ptr(p.cast()) };
        assert_eq!(s.to_str().unwrap(), "Killed");
    }

    // ---- strlcpy / strlcat --------------------------------------------------

    #[test]
    fn test_strlcpy_fits() {
        let src = b"hello\0";
        let mut dest = [0u8; 10];
        let ret = unsafe { strlcpy(dest.as_mut_ptr(), src.as_ptr(), dest.len()) };
        assert_eq!(ret, 5);
        assert_eq!(&dest[..6], b"hello\0");
    }

    #[test]
    fn test_strlcpy_truncates() {
        let src = b"hello world\0";
        let mut dest = [0u8; 5];
        let ret = unsafe { strlcpy(dest.as_mut_ptr(), src.as_ptr(), dest.len()) };
        assert_eq!(ret, 11); // full src length
        assert_eq!(&dest, b"hell\0");
    }

    #[test]
    fn test_strlcat_fits() {
        let mut buf = [0u8; 16];
        buf[..6].copy_from_slice(b"hello\0");
        let src = b" world\0";
        let ret = unsafe { strlcat(buf.as_mut_ptr(), src.as_ptr(), buf.len()) };
        assert_eq!(ret, 11);
        assert_eq!(&buf[..12], b"hello world\0");
    }

    // ---- strchrnul ----------------------------------------------------------

    #[test]
    fn test_strchrnul_found() {
        let s = b"hello\0";
        let p = unsafe { strchrnul(s.as_ptr(), b'l' as i32) };
        assert_eq!(p, unsafe { s.as_ptr().add(2) });
    }

    #[test]
    fn test_strchrnul_not_found_returns_nul() {
        let s = b"hello\0";
        let p = unsafe { strchrnul(s.as_ptr(), b'z' as i32) };
        assert_eq!(p, unsafe { s.as_ptr().add(5) }); // points to NUL
    }
}
