//! String-to-number conversion functions.
//!
//! Implements `strtol`, `strtoul`, `strtof`, `strtod`, and related functions
//! with correct POSIX semantics: whitespace skipping, sign handling, base
//! detection, overflow detection (ERANGE), and `endptr` update.
//!
//! All integer functions handle base 2–36 and auto-detection (base 0).
//! Float functions support decimal, hexadecimal (`0x1.8p+1`), `inf`, and `nan`.
//!
//! These functions are `pub` so that the `hadron-libc` staticlib shell can
//! re-export them and so that host-side unit tests can call them directly.

use crate::errno::{self, EINVAL, ERANGE};

// ---- C whitespace definition ------------------------------------------------

/// Returns `true` if `c` is a C whitespace character (matching `isspace`).
#[inline]
fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

// ---- Internal: digit value --------------------------------------------------

/// Returns the numeric value of an ASCII digit in the given base, or `None`.
#[inline]
fn digit_value(c: u8, base: u64) -> Option<u64> {
    let v = match c {
        b'0'..=b'9' => (c - b'0') as u64,
        b'a'..=b'z' => (c - b'a' + 10) as u64,
        b'A'..=b'Z' => (c - b'A' + 10) as u64,
        _ => return None,
    };
    if v < base { Some(v) } else { None }
}

// ---- Internal: parse unsigned integer ---------------------------------------

/// Core parser used by all integer conversion functions.
///
/// Advances `idx` past the consumed characters. Returns `(value, overflow)`.
///
/// # Safety
///
/// `s` must be a valid pointer; validity past `idx` is only assumed one byte
/// at a time (terminates at NUL or a non-digit).
unsafe fn parse_uint(s: *const u8, idx: &mut usize, base: u64) -> (u64, bool) {
    let mut val: u64 = 0;
    let mut overflow = false;

    loop {
        // SAFETY: We read one byte at a time; the caller guarantees at minimum
        // a NUL-terminated buffer reachable from s.
        let c = unsafe { *s.add(*idx) };
        let Some(d) = digit_value(c, base) else { break };
        match val.checked_mul(base).and_then(|v| v.checked_add(d)) {
            Some(v) => val = v,
            None => {
                overflow = true;
                val = u64::MAX;
                // Continue consuming digits without updating val.
                *idx += 1;
                loop {
                    let c2 = unsafe { *s.add(*idx) };
                    if digit_value(c2, base).is_none() {
                        break;
                    }
                    *idx += 1;
                }
                return (val, overflow);
            }
        }
        *idx += 1;
    }

    (val, overflow)
}

/// Parse base prefix and update `idx` and `base`.
///
/// Handles `0x`/`0X` for hex and leading `0` for octal when `base == 0`.
/// For `base == 16`, also strips an optional `0x`/`0X` prefix.
///
/// # Safety
///
/// `s` must be readable at `*idx` and `*idx + 1`.
unsafe fn resolve_base_and_prefix(s: *const u8, idx: &mut usize, base: i32) -> u64 {
    let mut b = base as u64;
    if b == 0 {
        // SAFETY: Caller ensures at least two bytes are readable (or NUL).
        if unsafe { *s.add(*idx) } == b'0' {
            let next = unsafe { *s.add(*idx + 1) };
            if next == b'x' || next == b'X' {
                b = 16;
                *idx += 2;
            } else {
                b = 8;
                *idx += 1;
            }
        } else {
            b = 10;
        }
    } else if b == 16 {
        if unsafe { *s.add(*idx) } == b'0' {
            let next = unsafe { *s.add(*idx + 1) };
            if next == b'x' || next == b'X' {
                *idx += 2;
            }
        }
    }
    b
}

// ---- strtoul / strtoull / strtoumax -----------------------------------------

/// `strtoul` — convert string to `unsigned long` (64-bit on LP64).
///
/// Parses the leading unsigned integer in `s` in the given `base` (2–36, or 0
/// for auto-detection). Sets `*endptr` to the first unconverted character.
/// On overflow, sets `errno` to `ERANGE` and returns `ULONG_MAX`.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoul(s: *const u8, endptr: *mut *const u8, base: i32) -> u64 {
    if s.is_null() {
        if !endptr.is_null() {
            // SAFETY: endptr is non-null per check above.
            unsafe { *endptr = s };
        }
        return 0;
    }

    let mut idx = 0usize;

    // Skip whitespace.
    // SAFETY: s is a valid NUL-terminated string.
    while is_c_space(unsafe { *s.add(idx) }) {
        idx += 1;
    }

    // Optional sign.
    let neg = unsafe { *s.add(idx) } == b'-';
    if neg || unsafe { *s.add(idx) } == b'+' {
        idx += 1;
    }

    let start = idx;

    // SAFETY: s is readable past idx (NUL-terminated).
    let base_u = unsafe { resolve_base_and_prefix(s, &mut idx, base) };

    if base_u < 2 || base_u > 36 {
        errno::set_errno(EINVAL);
        if !endptr.is_null() {
            // SAFETY: endptr non-null.
            unsafe { *endptr = s };
        }
        return 0;
    }

    // SAFETY: s is NUL-terminated; parse_uint terminates at NUL or non-digit.
    let (mut val, overflow) = unsafe { parse_uint(s, &mut idx, base_u) };

    // No digits consumed at all → reset endptr to start of non-whitespace.
    if idx == start {
        if !endptr.is_null() {
            // SAFETY: endptr non-null.
            unsafe { *endptr = s.add(start) };
        }
        return 0;
    }

    if overflow {
        errno::set_errno(ERANGE);
        val = u64::MAX;
    }

    // Negation of unsigned: wrap-around, same as glibc/musl.
    if neg {
        val = val.wrapping_neg();
    }

    if !endptr.is_null() {
        // SAFETY: endptr non-null.
        unsafe { *endptr = s.add(idx) };
    }
    val
}

/// `strtoull` — convert string to `unsigned long long` (alias for `strtoul` on LP64).
///
/// # Safety
///
/// Same as [`strtoul`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoull(s: *const u8, endptr: *mut *const u8, base: i32) -> u64 {
    // SAFETY: Same preconditions.
    unsafe { strtoul(s, endptr, base) }
}

/// `strtoumax` — convert string to `uintmax_t` (u64 on LP64).
///
/// # Safety
///
/// Same as [`strtoul`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoumax(s: *const u8, endptr: *mut *const u8, base: i32) -> u64 {
    // SAFETY: Same preconditions.
    unsafe { strtoul(s, endptr, base) }
}

// ---- strtol / strtoll / strtoimax -------------------------------------------

/// `strtol` — convert string to `long` (i64 on LP64).
///
/// On overflow, sets `errno` to `ERANGE` and returns `LONG_MAX` or `LONG_MIN`.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtol(s: *const u8, endptr: *mut *const u8, base: i32) -> i64 {
    if s.is_null() {
        if !endptr.is_null() {
            // SAFETY: endptr non-null.
            unsafe { *endptr = s };
        }
        return 0;
    }

    let mut idx = 0usize;

    // Skip whitespace.
    // SAFETY: s is NUL-terminated.
    while is_c_space(unsafe { *s.add(idx) }) {
        idx += 1;
    }

    let neg = unsafe { *s.add(idx) } == b'-';
    if neg || unsafe { *s.add(idx) } == b'+' {
        idx += 1;
    }

    let start = idx;

    // SAFETY: s is readable past idx.
    let base_u = unsafe { resolve_base_and_prefix(s, &mut idx, base) };

    if base_u < 2 || base_u > 36 {
        errno::set_errno(EINVAL);
        if !endptr.is_null() {
            // SAFETY: endptr non-null.
            unsafe { *endptr = s };
        }
        return 0;
    }

    // SAFETY: NUL-terminated string.
    let (uval, overflow) = unsafe { parse_uint(s, &mut idx, base_u) };

    if idx == start {
        if !endptr.is_null() {
            // SAFETY: endptr non-null.
            unsafe { *endptr = s.add(start) };
        }
        return 0;
    }

    if !endptr.is_null() {
        // SAFETY: endptr non-null.
        unsafe { *endptr = s.add(idx) };
    }

    if overflow || uval > i64::MAX as u64 + neg as u64 {
        errno::set_errno(ERANGE);
        return if neg { i64::MIN } else { i64::MAX };
    }

    if neg {
        (uval as i64).wrapping_neg()
    } else {
        uval as i64
    }
}

/// `strtoll` — convert string to `long long` (alias for `strtol` on LP64).
///
/// # Safety
///
/// Same as [`strtol`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoll(s: *const u8, endptr: *mut *const u8, base: i32) -> i64 {
    // SAFETY: Same preconditions.
    unsafe { strtol(s, endptr, base) }
}

/// `strtoimax` — convert string to `intmax_t` (i64 on LP64).
///
/// # Safety
///
/// Same as [`strtol`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoimax(s: *const u8, endptr: *mut *const u8, base: i32) -> i64 {
    // SAFETY: Same preconditions.
    unsafe { strtol(s, endptr, base) }
}

// ---- atoi / atol / atoll ----------------------------------------------------

/// `atoi` — convert string to `int`.
///
/// Equivalent to `(int)strtol(s, NULL, 10)`. No overflow detection.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atoi(s: *const u8) -> i32 {
    // SAFETY: s is NUL-terminated per caller contract.
    unsafe { strtol(s, core::ptr::null_mut(), 10) as i32 }
}

/// `atol` — convert string to `long`.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atol(s: *const u8) -> i64 {
    // SAFETY: s is NUL-terminated.
    unsafe { strtol(s, core::ptr::null_mut(), 10) }
}

/// `atoll` — convert string to `long long`.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atoll(s: *const u8) -> i64 {
    // SAFETY: s is NUL-terminated.
    unsafe { strtol(s, core::ptr::null_mut(), 10) }
}

// ---- Float parsing ----------------------------------------------------------

/// Parsed sign and exponent helper for float conversion.
struct FloatParts {
    neg: bool,
    mantissa: u64,
    /// Number of fractional digits absorbed into mantissa (base 10 or 16).
    frac_digits: u32,
    /// Base-10 or base-2 exponent from `e`/`p` notation.
    exp: i32,
    /// Whether the exponent uses base 2 (hex float) or base 10 (decimal).
    hex: bool,
}

/// Parse a decimal or hex floating-point string starting at `s[*idx..]`.
///
/// Returns `None` if no valid float was found (endptr left at start).
///
/// # Safety
///
/// `s` must be NUL-terminated and readable up to the point where parsing stops.
unsafe fn parse_float_parts(s: *const u8, idx: &mut usize) -> Option<FloatParts> {
    let start = *idx;

    let neg = unsafe { *s.add(*idx) } == b'-';
    if neg || unsafe { *s.add(*idx) } == b'+' {
        *idx += 1;
    }

    // Check for inf / nan.
    let c0 = unsafe { *s.add(*idx) };
    let c1 = unsafe { *s.add(*idx + 1) };
    let c2 = unsafe { *s.add(*idx + 2) };
    match (
        c0.to_ascii_lowercase(),
        c1.to_ascii_lowercase(),
        c2.to_ascii_lowercase(),
    ) {
        (b'i', b'n', b'f') => {
            *idx += 3;
            // Optionally consume "inity".
            let i4 = [
                unsafe { *s.add(*idx) }.to_ascii_lowercase(),
                unsafe { *s.add(*idx + 1) }.to_ascii_lowercase(),
                unsafe { *s.add(*idx + 2) }.to_ascii_lowercase(),
                unsafe { *s.add(*idx + 3) }.to_ascii_lowercase(),
                unsafe { *s.add(*idx + 4) }.to_ascii_lowercase(),
            ];
            if i4 == [b'i', b'n', b'i', b't', b'y'] {
                *idx += 5;
            }
            return Some(FloatParts {
                neg,
                mantissa: u64::MAX, // sentinel for infinity
                frac_digits: 0,
                exp: i32::MAX,
                hex: false,
            });
        }
        (b'n', b'a', b'n') => {
            *idx += 3;
            // Optionally consume (n-char-sequence).
            if unsafe { *s.add(*idx) } == b'(' {
                *idx += 1;
                while unsafe { *s.add(*idx) } != b')' && unsafe { *s.add(*idx) } != 0 {
                    *idx += 1;
                }
                if unsafe { *s.add(*idx) } == b')' {
                    *idx += 1;
                }
            }
            return Some(FloatParts {
                neg,
                mantissa: u64::MAX - 1, // sentinel for NaN
                frac_digits: 0,
                exp: i32::MAX - 1,
                hex: false,
            });
        }
        _ => {}
    }

    // Hex float: 0x / 0X prefix.
    let hex = unsafe { *s.add(*idx) } == b'0' && matches!(unsafe { *s.add(*idx + 1) }, b'x' | b'X');
    if hex {
        *idx += 2;
    }

    let digit_base: u64 = if hex { 16 } else { 10 };

    // Integer part.
    let mut mantissa: u64 = 0;
    let mut overflow = false;
    let mut has_digits = false;
    loop {
        let c = unsafe { *s.add(*idx) };
        let Some(d) = digit_value(c, digit_base) else {
            break;
        };
        has_digits = true;
        match mantissa
            .checked_mul(digit_base)
            .and_then(|v| v.checked_add(d))
        {
            Some(v) => mantissa = v,
            None => {
                overflow = true;
                mantissa = u64::MAX;
            }
        }
        *idx += 1;
    }

    // Fractional part.
    let mut frac_digits: u32 = 0;
    if unsafe { *s.add(*idx) } == b'.' {
        *idx += 1;
        loop {
            let c = unsafe { *s.add(*idx) };
            let Some(d) = digit_value(c, digit_base) else {
                break;
            };
            has_digits = true;
            if !overflow {
                match mantissa
                    .checked_mul(digit_base)
                    .and_then(|v| v.checked_add(d))
                {
                    Some(v) => {
                        mantissa = v;
                        frac_digits += 1;
                    }
                    None => overflow = true,
                }
            }
            *idx += 1;
        }
    }

    if !has_digits {
        // No digits consumed; roll back sign, restore idx.
        *idx = start;
        return None;
    }

    // Exponent.
    let exp_char = unsafe { *s.add(*idx) }.to_ascii_lowercase();
    let mut exp: i32 = 0;
    if (hex && exp_char == b'p') || (!hex && exp_char == b'e') {
        *idx += 1;
        let exp_neg = unsafe { *s.add(*idx) } == b'-';
        if exp_neg || unsafe { *s.add(*idx) } == b'+' {
            *idx += 1;
        }
        let mut exp_digits = false;
        while matches!(unsafe { *s.add(*idx) }, b'0'..=b'9') {
            exp_digits = true;
            let d = (unsafe { *s.add(*idx) } - b'0') as i32;
            exp = exp.saturating_mul(10).saturating_add(d);
            *idx += 1;
        }
        if !exp_digits {
            // Malformed exponent: back up.
            *idx -= 1; // undo sign/char
        }
        if exp_neg {
            exp = exp.wrapping_neg();
        }
    }

    Some(FloatParts {
        neg,
        mantissa,
        frac_digits,
        exp,
        hex,
    })
}

/// Combine parsed float parts into a `f64`.
fn assemble_f64(parts: &FloatParts) -> f64 {
    // Handle sentinels.
    if parts.exp == i32::MAX && parts.mantissa == u64::MAX {
        return if parts.neg {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if parts.exp == i32::MAX - 1 && parts.mantissa == u64::MAX - 1 {
        return f64::NAN;
    }

    let mut val = parts.mantissa as f64;
    if parts.hex {
        // Hex float: mantissa × 2^(exp − frac_digits × 4)
        let exp_adj = parts.exp - (parts.frac_digits as i32) * 4;
        val *= libm_pow2(exp_adj);
    } else {
        // Decimal float: mantissa × 10^(exp − frac_digits)
        let exp_adj = parts.exp - parts.frac_digits as i32;
        val *= libm_pow10(exp_adj);
    }
    if parts.neg { -val } else { val }
}

/// Compute `2.0f64.powi(e)` without libm.
#[inline]
fn libm_pow2(e: i32) -> f64 {
    if e == 0 {
        return 1.0;
    }
    let mut result = 1.0f64;
    let base = if e > 0 { 2.0f64 } else { 0.5f64 };
    let mut n = if e > 0 { e } else { -e };
    // Clamp to avoid denormals / infinity in the result.
    if n > 1074 {
        return if e > 0 { f64::INFINITY } else { 0.0 };
    }
    let mut b = base;
    while n > 0 {
        if n & 1 != 0 {
            result *= b;
        }
        b *= b;
        n >>= 1;
    }
    result
}

/// Compute `10.0f64.powi(e)` without libm.
#[inline]
fn libm_pow10(e: i32) -> f64 {
    if e == 0 {
        return 1.0;
    }
    // Clamp: 10^309 > f64::MAX, 10^-324 < f64::MIN_POSITIVE.
    if e > 308 {
        return f64::INFINITY;
    }
    if e < -323 {
        return 0.0;
    }
    let mut result = 1.0f64;
    let base = if e > 0 { 10.0f64 } else { 0.1f64 };
    let mut n = if e > 0 { e } else { -e };
    let mut b = base;
    while n > 0 {
        if n & 1 != 0 {
            result *= b;
        }
        b *= b;
        n >>= 1;
    }
    result
}

/// `strtod` — convert string to `double`.
///
/// Handles decimal floats, hex floats (`0x1.8p+1`), `inf`, and `nan`.
/// On overflow, sets `errno` to `ERANGE` and returns `HUGE_VAL`.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtod(s: *const u8, endptr: *mut *const u8) -> f64 {
    if s.is_null() {
        if !endptr.is_null() {
            // SAFETY: endptr non-null.
            unsafe { *endptr = s };
        }
        return 0.0;
    }

    let mut idx = 0usize;
    // SAFETY: s is NUL-terminated.
    while is_c_space(unsafe { *s.add(idx) }) {
        idx += 1;
    }

    let start = idx;
    // SAFETY: NUL-terminated, parse stops at NUL or non-float char.
    let parts = unsafe { parse_float_parts(s, &mut idx) };

    let val = match parts {
        None => {
            if !endptr.is_null() {
                // SAFETY: endptr non-null.
                unsafe { *endptr = s.add(start) };
            }
            return 0.0;
        }
        Some(ref p) => assemble_f64(p),
    };

    if val.is_infinite() {
        errno::set_errno(ERANGE);
    }

    if !endptr.is_null() {
        // SAFETY: endptr non-null.
        unsafe { *endptr = s.add(idx) };
    }
    val
}

/// `strtof` — convert string to `float`.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtof(s: *const u8, endptr: *mut *const u8) -> f32 {
    // SAFETY: Same preconditions forwarded.
    let d = unsafe { strtod(s, endptr) };
    d as f32
}

/// `strtold` — convert string to `long double` (treated as `f64` on x86-64 without SSE80).
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `endptr` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtold(s: *const u8, endptr: *mut *const u8) -> f64 {
    // SAFETY: Same preconditions forwarded.
    unsafe { strtod(s, endptr) }
}

// ---- Integer arithmetic helpers (C99 stdlib.h) ------------------------------

/// `llabs` — absolute value of `long long`.
#[unsafe(no_mangle)]
pub extern "C" fn llabs(x: i64) -> i64 {
    if x < 0 { x.wrapping_neg() } else { x }
}

/// `imaxabs` — absolute value of `intmax_t` (i64 on LP64).
#[unsafe(no_mangle)]
pub extern "C" fn imaxabs(x: i64) -> i64 {
    llabs(x)
}

/// Result of `div()`.
#[repr(C)]
pub struct DivT {
    pub quot: i32,
    pub rem: i32,
}

/// Result of `ldiv()`.
#[repr(C)]
pub struct LdivT {
    pub quot: i64,
    pub rem: i64,
}

/// Result of `lldiv()`.
#[repr(C)]
pub struct LldivT {
    pub quot: i64,
    pub rem: i64,
}

/// `div` — quotient and remainder of `numer / denom`.
#[unsafe(no_mangle)]
pub extern "C" fn div(numer: i32, denom: i32) -> DivT {
    DivT {
        quot: numer / denom,
        rem: numer % denom,
    }
}

/// `ldiv` — quotient and remainder of `numer / denom` (long).
#[unsafe(no_mangle)]
pub extern "C" fn ldiv(numer: i64, denom: i64) -> LdivT {
    LdivT {
        quot: numer / denom,
        rem: numer % denom,
    }
}

/// `lldiv` — quotient and remainder of `numer / denom` (long long).
#[unsafe(no_mangle)]
pub extern "C" fn lldiv(numer: i64, denom: i64) -> LldivT {
    LldivT {
        quot: numer / denom,
        rem: numer % denom,
    }
}

/// `imaxdiv` — quotient and remainder of `numer / denom` (intmax_t).
#[unsafe(no_mangle)]
pub extern "C" fn imaxdiv(numer: i64, denom: i64) -> LldivT {
    lldiv(numer, denom)
}

// ---- Unit tests -------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: call `strtol` on a Rust string literal, return `(value, endptr_offset)`.
    fn do_strtol(s: &[u8], base: i32) -> (i64, usize) {
        let mut end: *const u8 = core::ptr::null();
        let val = unsafe { strtol(s.as_ptr(), &mut end, base) };
        let off = unsafe { end.offset_from(s.as_ptr()) } as usize;
        (val, off)
    }

    fn do_strtoul(s: &[u8], base: i32) -> (u64, usize) {
        let mut end: *const u8 = core::ptr::null();
        let val = unsafe { strtoul(s.as_ptr(), &mut end, base) };
        let off = unsafe { end.offset_from(s.as_ptr()) } as usize;
        (val, off)
    }

    fn do_strtod(s: &[u8]) -> (f64, usize) {
        let mut end: *const u8 = core::ptr::null();
        let val = unsafe { strtod(s.as_ptr(), &mut end) };
        let off = unsafe { end.offset_from(s.as_ptr()) } as usize;
        (val, off)
    }

    // ---- strtol -------------------------------------------------------------

    #[test]
    fn strtol_basic_decimal() {
        let (v, off) = do_strtol(b"42\0", 10);
        assert_eq!(v, 42);
        assert_eq!(off, 2);
    }

    #[test]
    fn strtol_negative() {
        let (v, _) = do_strtol(b"-1234\0", 10);
        assert_eq!(v, -1234);
    }

    #[test]
    fn strtol_hex_prefix() {
        let (v, off) = do_strtol(b"0xff\0", 0);
        assert_eq!(v, 255);
        assert_eq!(off, 4);
    }

    #[test]
    fn strtol_octal_prefix() {
        let (v, _) = do_strtol(b"010\0", 0);
        assert_eq!(v, 8);
    }

    #[test]
    fn strtol_base16_explicit() {
        let (v, _) = do_strtol(b"ff\0", 16);
        assert_eq!(v, 255);
    }

    #[test]
    fn strtol_leading_whitespace() {
        let (v, _) = do_strtol(b"  \t42\0", 10);
        assert_eq!(v, 42);
    }

    #[test]
    fn strtol_stops_at_non_digit() {
        let (v, off) = do_strtol(b"42abc\0", 10);
        assert_eq!(v, 42);
        assert_eq!(off, 2);
    }

    #[test]
    fn strtol_overflow_positive() {
        // 2^63 overflows i64.
        let (v, _) = do_strtol(b"9999999999999999999\0", 10);
        assert_eq!(v, i64::MAX);
    }

    #[test]
    fn strtol_overflow_negative() {
        let (v, _) = do_strtol(b"-9999999999999999999\0", 10);
        assert_eq!(v, i64::MIN);
    }

    #[test]
    fn strtol_no_digits() {
        let (v, off) = do_strtol(b"abc\0", 10);
        assert_eq!(v, 0);
        assert_eq!(off, 0); // endptr points to 'a'
    }

    // ---- strtoul ------------------------------------------------------------

    #[test]
    fn strtoul_basic() {
        let (v, _) = do_strtoul(b"255\0", 10);
        assert_eq!(v, 255);
    }

    #[test]
    fn strtoul_hex() {
        let (v, _) = do_strtoul(b"0xdeadbeef\0", 0);
        assert_eq!(v, 0xdead_beef);
    }

    #[test]
    fn strtoul_overflow() {
        let (v, _) = do_strtoul(b"99999999999999999999\0", 10);
        assert_eq!(v, u64::MAX);
    }

    // ---- atoi / atol --------------------------------------------------------

    #[test]
    fn atoi_basic() {
        let v = unsafe { atoi(b"123\0".as_ptr()) };
        assert_eq!(v, 123);
    }

    #[test]
    fn atoi_negative() {
        let v = unsafe { atoi(b"-99\0".as_ptr()) };
        assert_eq!(v, -99);
    }

    #[test]
    fn atol_large() {
        let v = unsafe { atol(b"1000000000\0".as_ptr()) };
        assert_eq!(v, 1_000_000_000);
    }

    #[test]
    fn atoll_basic() {
        let v = unsafe { atoll(b"9876543210\0".as_ptr()) };
        assert_eq!(v, 9_876_543_210);
    }

    // ---- strtod -------------------------------------------------------------

    #[test]
    fn strtod_basic() {
        let (v, _) = do_strtod(b"3.14\0");
        assert!((v - 3.14).abs() < 0.001);
    }

    #[test]
    fn strtod_negative() {
        let (v, _) = do_strtod(b"-2.5\0");
        assert_eq!(v, -2.5);
    }

    #[test]
    fn strtod_exponent() {
        let (v, _) = do_strtod(b"1.5e2\0");
        assert!((v - 150.0).abs() < 0.001);
    }

    #[test]
    fn strtod_negative_exponent() {
        let (v, _) = do_strtod(b"1.5e-2\0");
        assert!((v - 0.015).abs() < 0.0001);
    }

    #[test]
    fn strtod_infinity() {
        let (v, _) = do_strtod(b"inf\0");
        assert!(v.is_infinite() && v > 0.0);

        let (v2, _) = do_strtod(b"-infinity\0");
        assert!(v2.is_infinite() && v2 < 0.0);
    }

    #[test]
    fn strtod_nan() {
        let (v, _) = do_strtod(b"nan\0");
        assert!(v.is_nan());
    }

    #[test]
    fn strtod_hex_float() {
        // 0x1.8p+1 = 1.5 × 2 = 3.0
        let (v, _) = do_strtod(b"0x1.8p+1\0");
        assert!((v - 3.0).abs() < 0.001, "got {v}");
    }

    #[test]
    fn strtod_stops_at_non_digit() {
        let (v, off) = do_strtod(b"1.5rest\0");
        assert_eq!(v, 1.5);
        assert_eq!(off, 3);
    }

    #[test]
    fn strtod_no_input() {
        let (v, off) = do_strtod(b"abc\0");
        assert_eq!(v, 0.0);
        assert_eq!(off, 0);
    }
}
