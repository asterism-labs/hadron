//! Locale functions.
//!
//! Hadron only supports the "C" / "POSIX" locale.
//! POSIX functions: `setlocale`, `localeconv`.

/// Locale categories (values match glibc).
pub const LC_CTYPE: i32 = 0;
pub const LC_NUMERIC: i32 = 1;
pub const LC_TIME: i32 = 2;
pub const LC_COLLATE: i32 = 3;
pub const LC_MONETARY: i32 = 4;
pub const LC_MESSAGES: i32 = 5;
pub const LC_ALL: i32 = 6;

static C_LOCALE: &[u8] = b"C\0";

/// `struct lconv` — numeric formatting information.
#[repr(C)]
pub struct Lconv {
    pub decimal_point: *const u8,
    pub thousands_sep: *const u8,
    pub grouping: *const u8,
    pub int_curr_symbol: *const u8,
    pub currency_symbol: *const u8,
    pub mon_decimal_point: *const u8,
    pub mon_thousands_sep: *const u8,
    pub mon_grouping: *const u8,
    pub positive_sign: *const u8,
    pub negative_sign: *const u8,
}

// SAFETY: All pointers point to static data.
unsafe impl Sync for Lconv {}

static C_LCONV: Lconv = Lconv {
    decimal_point: b".\0".as_ptr(),
    thousands_sep: b"\0".as_ptr(),
    grouping: b"\0".as_ptr(),
    int_curr_symbol: b"\0".as_ptr(),
    currency_symbol: b"\0".as_ptr(),
    mon_decimal_point: b"\0".as_ptr(),
    mon_thousands_sep: b"\0".as_ptr(),
    mon_grouping: b"\0".as_ptr(),
    positive_sign: b"\0".as_ptr(),
    negative_sign: b"-\0".as_ptr(),
};

/// Set or query the program's locale.
///
/// Only "C", "POSIX", and "" (default) are accepted.
///
/// # Safety
///
/// `locale` must be null (query) or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setlocale(_category: i32, locale: *const u8) -> *const u8 {
    if locale.is_null() {
        // Query: return current locale.
        return C_LOCALE.as_ptr();
    }
    // SAFETY: locale is NUL-terminated.
    let first = unsafe { *locale };
    // Accept "", "C", "POSIX"
    if first == 0 || first == b'C' || first == b'P' {
        C_LOCALE.as_ptr()
    } else {
        core::ptr::null()
    }
}

/// Return the numeric formatting conventions of the current locale.
#[unsafe(no_mangle)]
pub extern "C" fn localeconv() -> *const Lconv {
    &C_LCONV
}
