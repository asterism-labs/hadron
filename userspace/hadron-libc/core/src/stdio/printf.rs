//! Printf format engine.
//!
//! Implements a Rust state-machine parser for printf format strings.
//! Supports: `%d`, `%i`, `%u`, `%x`, `%X`, `%o`, `%s`, `%c`, `%p`,
//! `%ld`, `%lu`, `%lx`, `%lld`, `%llu`, `%llx`, `%n`, `%%`.
//! Modifiers: width, precision, `-` (left-justify), `0` (zero-pad), `+`, space.
//! NO floating point in Phase 1 (SSE disabled in user target).

use super::{FILE, fwrite};

/// Core format engine: parse `fmt` and emit bytes through `writer`.
///
/// `next_arg` returns the next variadic argument as a `usize`. On x86_64
/// with soft-float (no SSE), every C variadic argument occupies one 8-byte
/// slot regardless of declared type, so a single `usize` reader suffices.
pub fn format_to(
    writer: &mut dyn FnMut(&[u8]),
    fmt: *const u8,
    next_arg: &mut dyn FnMut() -> usize,
) {
    let mut i = 0;
    loop {
        // SAFETY: fmt is a NUL-terminated string.
        let ch = unsafe { *fmt.add(i) };
        if ch == 0 {
            break;
        }
        if ch != b'%' {
            writer(&[ch]);
            i += 1;
            continue;
        }
        i += 1; // skip '%'

        // Parse flags.
        let mut left_justify = false;
        let mut zero_pad = false;
        let mut sign_plus = false;
        let mut sign_space = false;
        let mut alt_form = false;
        loop {
            // SAFETY: fmt is NUL-terminated.
            match unsafe { *fmt.add(i) } {
                b'-' => {
                    left_justify = true;
                    i += 1;
                }
                b'0' => {
                    zero_pad = true;
                    i += 1;
                }
                b'+' => {
                    sign_plus = true;
                    i += 1;
                }
                b' ' => {
                    sign_space = true;
                    i += 1;
                }
                b'#' => {
                    alt_form = true;
                    i += 1;
                }
                _ => break,
            }
        }
        if left_justify {
            zero_pad = false;
        }

        // Parse width.
        let mut width: usize = 0;
        let mut width_from_arg = false;
        // SAFETY: fmt is NUL-terminated.
        if unsafe { *fmt.add(i) } == b'*' {
            width_from_arg = true;
            i += 1;
        } else {
            while matches!(unsafe { *fmt.add(i) }, b'0'..=b'9') {
                width = width * 10 + (unsafe { *fmt.add(i) } - b'0') as usize;
                i += 1;
            }
        }

        // Parse precision.
        let mut precision: Option<usize> = None;
        // SAFETY: fmt is NUL-terminated.
        if unsafe { *fmt.add(i) } == b'.' {
            i += 1;
            if unsafe { *fmt.add(i) } == b'*' {
                precision = Some(next_arg());
                i += 1;
            } else {
                let mut prec: usize = 0;
                while matches!(unsafe { *fmt.add(i) }, b'0'..=b'9') {
                    prec = prec * 10 + (unsafe { *fmt.add(i) } - b'0') as usize;
                    i += 1;
                }
                precision = Some(prec);
            }
        }

        if width_from_arg {
            width = next_arg();
        }

        // Parse length modifier.
        let mut _long_count: u8 = 0;
        loop {
            // SAFETY: fmt is NUL-terminated.
            match unsafe { *fmt.add(i) } {
                b'l' => {
                    _long_count += 1;
                    i += 1;
                }
                b'h' | b'z' | b'j' | b't' => {
                    i += 1; // consume but treat as default
                }
                _ => break,
            }
        }

        // Parse conversion specifier.
        // SAFETY: fmt is NUL-terminated.
        let spec = unsafe { *fmt.add(i) };
        i += 1;

        match spec {
            b'%' => writer(b"%"),
            b'd' | b'i' => {
                let val = next_arg() as isize;
                let mut buf = [0u8; 24];
                let s = format_signed(val, &mut buf, sign_plus, sign_space);
                write_padded(
                    writer,
                    s,
                    width,
                    left_justify,
                    zero_pad,
                    s.first() == Some(&b'-')
                        || s.first() == Some(&b'+')
                        || s.first() == Some(&b' '),
                );
            }
            b'u' => {
                let val = next_arg();
                let mut buf = [0u8; 24];
                let s = format_unsigned(val, 10, false, &mut buf);
                write_padded(writer, s, width, left_justify, zero_pad, false);
            }
            b'x' => {
                let val = next_arg();
                let mut buf = [0u8; 24];
                let s = format_unsigned(val, 16, false, &mut buf);
                if alt_form && val != 0 {
                    writer(b"0x");
                }
                write_padded(writer, s, width, left_justify, zero_pad, false);
            }
            b'X' => {
                let val = next_arg();
                let mut buf = [0u8; 24];
                let s = format_unsigned(val, 16, true, &mut buf);
                if alt_form && val != 0 {
                    writer(b"0X");
                }
                write_padded(writer, s, width, left_justify, zero_pad, false);
            }
            b'o' => {
                let val = next_arg();
                let mut buf = [0u8; 24];
                let s = format_unsigned(val, 8, false, &mut buf);
                if alt_form && val != 0 {
                    writer(b"0");
                }
                write_padded(writer, s, width, left_justify, zero_pad, false);
            }
            b's' => {
                let ptr = next_arg() as *const u8;
                if ptr.is_null() {
                    let s = b"(null)";
                    let len = match precision {
                        Some(p) => p.min(s.len()),
                        None => s.len(),
                    };
                    write_padded(writer, &s[..len], width, left_justify, false, false);
                } else {
                    // SAFETY: Caller guarantees valid NUL-terminated string.
                    let slen = unsafe { crate::string::strlen(ptr) };
                    let len = match precision {
                        Some(p) => p.min(slen),
                        None => slen,
                    };
                    let s = unsafe { core::slice::from_raw_parts(ptr, len) };
                    write_padded(writer, s, width, left_justify, false, false);
                }
            }
            b'c' => {
                let ch = next_arg() as u8;
                write_padded(writer, &[ch], width, left_justify, false, false);
            }
            b'p' => {
                let val = next_arg();
                if val == 0 {
                    write_padded(writer, b"(nil)", width, left_justify, false, false);
                } else {
                    writer(b"0x");
                    let mut buf = [0u8; 24];
                    let s = format_unsigned(val, 16, false, &mut buf);
                    writer(s);
                }
            }
            b'n' => {
                // %n is a security risk; we intentionally do nothing.
            }
            _ => {
                // Unknown specifier, output literally.
                writer(&[b'%', spec]);
            }
        }
    }
}

// ---- Formatting helpers ------------------------------------------------------

fn format_signed<'a>(val: isize, buf: &'a mut [u8; 24], plus: bool, space: bool) -> &'a [u8] {
    let (neg, abs) = if val < 0 {
        (true, (-(val as i128)) as usize)
    } else {
        (false, val as usize)
    };

    let mut pos = buf.len();
    if abs == 0 {
        pos -= 1;
        buf[pos] = b'0';
    } else {
        let mut v = abs;
        while v > 0 {
            pos -= 1;
            buf[pos] = b'0' + (v % 10) as u8;
            v /= 10;
        }
    }

    if neg {
        pos -= 1;
        buf[pos] = b'-';
    } else if plus {
        pos -= 1;
        buf[pos] = b'+';
    } else if space {
        pos -= 1;
        buf[pos] = b' ';
    }

    &buf[pos..]
}

fn format_unsigned<'a>(val: usize, base: usize, upper: bool, buf: &'a mut [u8; 24]) -> &'a [u8] {
    let digits = if upper {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };

    let mut pos = buf.len();
    if val == 0 {
        pos -= 1;
        buf[pos] = b'0';
    } else {
        let mut v = val;
        while v > 0 {
            pos -= 1;
            buf[pos] = digits[v % base];
            v /= base;
        }
    }

    &buf[pos..]
}

fn write_padded(
    writer: &mut dyn FnMut(&[u8]),
    content: &[u8],
    width: usize,
    left_justify: bool,
    zero_pad: bool,
    has_sign: bool,
) {
    let pad_char = if zero_pad { b'0' } else { b' ' };
    let pad_len = width.saturating_sub(content.len());

    if zero_pad && has_sign && !content.is_empty() {
        // Write sign first, then zero-pad, then digits.
        writer(&content[..1]);
        for _ in 0..pad_len {
            writer(&[pad_char]);
        }
        writer(&content[1..]);
    } else if left_justify {
        writer(content);
        for _ in 0..pad_len {
            writer(&[b' ']);
        }
    } else {
        for _ in 0..pad_len {
            writer(&[pad_char]);
        }
        writer(content);
    }
}

// ---- C ABI printf family -----------------------------------------------------
//
// The `...` parameter gives us a `VaListImpl` (unnamed type) that supports
// `.arg::<T>()`. We feed each arg into `format_to` via a closure, avoiding
// any need to name `VaListImpl` explicitly.

/// `printf(fmt, ...)` — print formatted output to stdout.
///
/// # Safety
///
/// `fmt` must be a valid NUL-terminated format string. Variadic arguments
/// must match the format specifiers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn printf(fmt: *const u8, mut args: ...) -> i32 {
    let stdout = super::__stdout();
    let mut count: i32 = 0;

    let mut writer = |bytes: &[u8]| {
        // SAFETY: stdout is valid static, bytes is a valid slice.
        unsafe { fwrite(bytes.as_ptr(), 1, bytes.len(), stdout) };
        count += bytes.len() as i32;
    };

    // SAFETY: Caller guarantees args match format specifiers.
    let mut next_arg = || -> usize { unsafe { args.arg::<usize>() } };

    format_to(&mut writer, fmt, &mut next_arg);
    count
}

/// `fprintf(stream, fmt, ...)` — print formatted to a stream.
///
/// # Safety
///
/// `stream` and `fmt` must be valid. Variadic arguments must match format.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fprintf(stream: *mut FILE, fmt: *const u8, mut args: ...) -> i32 {
    let mut count: i32 = 0;
    let mut writer = |bytes: &[u8]| {
        // SAFETY: Caller guarantees stream is valid.
        unsafe { fwrite(bytes.as_ptr(), 1, bytes.len(), stream) };
        count += bytes.len() as i32;
    };

    // SAFETY: Caller guarantees args match format specifiers.
    let mut next_arg = || -> usize { unsafe { args.arg::<usize>() } };

    format_to(&mut writer, fmt, &mut next_arg);
    count
}

/// `snprintf(buf, size, fmt, ...)` — print formatted to a buffer.
///
/// # Safety
///
/// `buf` must be valid for `size` bytes. `fmt` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn snprintf(buf: *mut u8, size: usize, fmt: *const u8, mut args: ...) -> i32 {
    let mut pos: usize = 0;
    let mut total: usize = 0;

    let mut writer = |bytes: &[u8]| {
        total += bytes.len();
        if !buf.is_null() && size > 0 {
            for &b in bytes {
                if pos < size - 1 {
                    // SAFETY: pos < size - 1, buf valid for size bytes.
                    unsafe { *buf.add(pos) = b };
                    pos += 1;
                }
            }
        }
    };

    // SAFETY: Caller guarantees args match format specifiers.
    let mut next_arg = || -> usize { unsafe { args.arg::<usize>() } };

    format_to(&mut writer, fmt, &mut next_arg);

    // NUL-terminate.
    if !buf.is_null() && size > 0 {
        let term_pos = pos.min(size - 1);
        // SAFETY: term_pos < size, buf valid for size bytes.
        unsafe { *buf.add(term_pos) = 0 };
    }

    total as i32
}

/// `sprintf(buf, fmt, ...)` — print formatted to a buffer (no size limit).
///
/// # Safety
///
/// `buf` must have sufficient space. `fmt` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sprintf(buf: *mut u8, fmt: *const u8, mut args: ...) -> i32 {
    let mut pos: usize = 0;

    let mut writer = |bytes: &[u8]| {
        if !buf.is_null() {
            for &b in bytes {
                // SAFETY: Caller guarantees sufficient space.
                unsafe { *buf.add(pos) = b };
                pos += 1;
            }
        }
    };

    // SAFETY: Caller guarantees args match format specifiers.
    let mut next_arg = || -> usize { unsafe { args.arg::<usize>() } };

    format_to(&mut writer, fmt, &mut next_arg);

    // NUL-terminate.
    if !buf.is_null() {
        // SAFETY: Caller guarantees sufficient space.
        unsafe { *buf.add(pos) = 0 };
    }

    pos as i32
}

/// `vsnprintf(buf, size, fmt, ap)` — formatted print to buffer with va_list.
///
/// # Safety
///
/// `buf` must be valid for `size` bytes. `fmt` must be NUL-terminated.
/// `args` must be a valid va_list matching the format specifiers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vsnprintf(
    buf: *mut u8,
    size: usize,
    fmt: *const u8,
    mut args: core::ffi::VaList<'_>,
) -> i32 {
    let mut pos: usize = 0;
    let mut total: usize = 0;

    let mut writer = |bytes: &[u8]| {
        total += bytes.len();
        if !buf.is_null() && size > 0 {
            for &b in bytes {
                if pos < size - 1 {
                    // SAFETY: pos < size - 1, buf valid for size bytes.
                    unsafe { *buf.add(pos) = b };
                    pos += 1;
                }
            }
        }
    };

    // SAFETY: Caller guarantees args match format specifiers.
    let mut next_arg = || -> usize { unsafe { args.arg::<usize>() } };
    format_to(&mut writer, fmt, &mut next_arg);

    if !buf.is_null() && size > 0 {
        let term_pos = pos.min(size - 1);
        // SAFETY: term_pos < size, buf valid for size bytes.
        unsafe { *buf.add(term_pos) = 0 };
    }
    total as i32
}

/// `vsprintf(buf, fmt, ap)` — formatted print to buffer (no size limit) with va_list.
///
/// # Safety
///
/// `buf` must have sufficient space. `fmt` must be NUL-terminated.
/// `args` must be a valid va_list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vsprintf(
    buf: *mut u8,
    fmt: *const u8,
    mut args: core::ffi::VaList<'_>,
) -> i32 {
    let mut pos: usize = 0;
    let mut writer = |bytes: &[u8]| {
        if !buf.is_null() {
            for &b in bytes {
                // SAFETY: Caller guarantees sufficient space.
                unsafe { *buf.add(pos) = b };
                pos += 1;
            }
        }
    };
    // SAFETY: Caller guarantees args match format specifiers.
    let mut next_arg = || -> usize { unsafe { args.arg::<usize>() } };
    format_to(&mut writer, fmt, &mut next_arg);
    if !buf.is_null() {
        // SAFETY: Caller guarantees sufficient space.
        unsafe { *buf.add(pos) = 0 };
    }
    pos as i32
}

/// `vfprintf(stream, fmt, ap)` — formatted print to stream with va_list.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer. `fmt` must be NUL-terminated.
/// `args` must be a valid va_list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vfprintf(
    stream: *mut FILE,
    fmt: *const u8,
    mut args: core::ffi::VaList<'_>,
) -> i32 {
    let mut count: i32 = 0;
    let mut writer = |bytes: &[u8]| {
        // SAFETY: Caller guarantees stream is valid.
        unsafe { fwrite(bytes.as_ptr(), 1, bytes.len(), stream) };
        count += bytes.len() as i32;
    };
    // SAFETY: Caller guarantees args match format specifiers.
    let mut next_arg = || -> usize { unsafe { args.arg::<usize>() } };
    format_to(&mut writer, fmt, &mut next_arg);
    count
}

/// `vprintf(fmt, ap)` — formatted print to stdout with va_list.
///
/// # Safety
///
/// `fmt` must be NUL-terminated. `args` must be a valid va_list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vprintf(fmt: *const u8, args: core::ffi::VaList<'_>) -> i32 {
    let stdout = super::__stdout();
    // SAFETY: stdout is always valid; fmt is NUL-terminated (caller guarantee).
    unsafe { vfprintf(stdout, fmt, args) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format_test(fmt_str: &[u8], setup: impl FnOnce(&mut Vec<usize>)) -> Vec<u8> {
        let mut args_data = Vec::new();
        setup(&mut args_data);
        let mut output = Vec::new();
        let mut idx = 0;
        let mut next_arg = || -> usize {
            let val = args_data[idx];
            idx += 1;
            val
        };
        format_to(
            &mut |bytes: &[u8]| output.extend_from_slice(bytes),
            fmt_str.as_ptr(),
            &mut next_arg,
        );
        output
    }

    #[test]
    fn test_plain_text() {
        let out = format_test(b"hello world\0", |_| {});
        assert_eq!(&out, b"hello world");
    }

    #[test]
    fn test_percent_d() {
        let out = format_test(b"%d\0", |args| args.push(42usize));
        assert_eq!(&out, b"42");
    }

    #[test]
    fn test_percent_d_negative() {
        let out = format_test(b"%d\0", |args| args.push((-7isize) as usize));
        assert_eq!(&out, b"-7");
    }

    #[test]
    fn test_percent_x() {
        let out = format_test(b"%x\0", |args| args.push(255usize));
        assert_eq!(&out, b"ff");
    }

    #[test]
    fn test_percent_upper_x() {
        let out = format_test(b"%X\0", |args| args.push(255usize));
        assert_eq!(&out, b"FF");
    }

    #[test]
    fn test_percent_s() {
        let s = b"hello\0";
        let out = format_test(b"%s\0", |args| args.push(s.as_ptr() as usize));
        assert_eq!(&out, b"hello");
    }

    #[test]
    fn test_percent_s_null() {
        let out = format_test(b"%s\0", |args| args.push(0usize));
        assert_eq!(&out, b"(null)");
    }

    #[test]
    fn test_percent_c() {
        let out = format_test(b"%c\0", |args| args.push(b'A' as usize));
        assert_eq!(&out, b"A");
    }

    #[test]
    fn test_width_right_pad() {
        let out = format_test(b"%10d\0", |args| args.push(42usize));
        assert_eq!(&out, b"        42");
    }

    #[test]
    fn test_width_left_pad() {
        let out = format_test(b"%-10d\0", |args| args.push(42usize));
        assert_eq!(&out, b"42        ");
    }

    #[test]
    fn test_zero_pad() {
        let out = format_test(b"%05d\0", |args| args.push(42usize));
        assert_eq!(&out, b"00042");
    }

    #[test]
    fn test_percent_percent() {
        let out = format_test(b"100%%\0", |_| {});
        assert_eq!(&out, b"100%");
    }

    #[test]
    fn test_mixed() {
        let s = b"world\0";
        let out = format_test(b"hello %s %d\0", |args| {
            args.push(s.as_ptr() as usize);
            args.push(42usize);
        });
        assert_eq!(&out, b"hello world 42");
    }

    #[test]
    fn test_percent_u() {
        let out = format_test(b"%u\0", |args| args.push(4294967295usize));
        // On 64-bit, this is just the number.
        assert!(out.starts_with(b"4294967295"));
    }

    #[test]
    fn test_percent_o() {
        let out = format_test(b"%o\0", |args| args.push(8usize));
        assert_eq!(&out, b"10");
    }

    #[test]
    fn test_precision_string() {
        let s = b"hello world\0";
        let out = format_test(b"%.5s\0", |args| args.push(s.as_ptr() as usize));
        assert_eq!(&out, b"hello");
    }

    #[test]
    fn test_alt_hex() {
        let out = format_test(b"%#x\0", |args| args.push(255usize));
        assert_eq!(&out, b"0xff");
    }

    #[test]
    fn test_sign_plus() {
        let out = format_test(b"%+d\0", |args| args.push(42usize));
        assert_eq!(&out, b"+42");
    }
}
