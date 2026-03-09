//! utest: POSIX search algorithm regression tests.
//!
//! Covers: `qsort` (integer array), `bsearch` (binary search in sorted array).

#![no_std]
#![no_main]

// Force hadron_libc_core to be linked so its #[no_mangle] symbols (qsort,
// bsearch) are available to the `extern "C"` declarations below.
extern crate hadron_libc_core;

use hadron_utest::utest_main;

utest_main!(
    test_qsort_integers,
    test_bsearch_found,
    test_bsearch_not_found,
);

// ── extern declarations ───────────────────────────────────────────────────────

unsafe extern "C" {
    fn qsort(
        base: *mut u8,
        nmemb: usize,
        size: usize,
        compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
    );
    fn bsearch(
        key: *const u8,
        base: *const u8,
        nmemb: usize,
        size: usize,
        compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
    ) -> *mut u8;
}

// ── comparators ──────────────────────────────────────────────────────────────

/// Comparator for `i32` values used with `qsort` / `bsearch`.
///
/// # Safety
///
/// `a` and `b` must point to valid `i32` values.
unsafe extern "C" fn cmp_i32(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: caller guarantees a and b point to valid i32 values.
    let (va, vb) = unsafe { (*(a.cast::<i32>()), *(b.cast::<i32>())) };
    va.cmp(&vb) as i32
}

// ── tests ─────────────────────────────────────────────────────────────────────

fn test_qsort_integers() {
    let mut arr: [i32; 7] = [5, 3, 8, 1, 9, 2, 7];
    // SAFETY: arr is a valid slice of 7 i32s; cmp_i32 is a valid comparator.
    unsafe {
        qsort(
            arr.as_mut_ptr().cast::<u8>(),
            7,
            core::mem::size_of::<i32>(),
            cmp_i32,
        );
    }
    assert_eq!(arr, [1, 2, 3, 5, 7, 8, 9]);
}

fn test_bsearch_found() {
    let arr: [i32; 5] = [1, 3, 5, 7, 9];
    let key: i32 = 5;
    // SAFETY: arr is a valid sorted slice; key and arr elements are i32s.
    let result = unsafe {
        bsearch(
            (&raw const key).cast::<u8>(),
            arr.as_ptr().cast::<u8>(),
            5,
            core::mem::size_of::<i32>(),
            cmp_i32,
        )
    };
    assert!(!result.is_null());
    // SAFETY: result points within arr, which holds i32 values.
    let found = unsafe { *(result.cast::<i32>()) };
    assert_eq!(found, 5);
}

fn test_bsearch_not_found() {
    let arr: [i32; 5] = [1, 3, 5, 7, 9];
    let key: i32 = 4;
    // SAFETY: arr is a valid sorted slice; key and arr elements are i32s.
    let result = unsafe {
        bsearch(
            (&raw const key).cast::<u8>(),
            arr.as_ptr().cast::<u8>(),
            5,
            core::mem::size_of::<i32>(),
            cmp_i32,
        )
    };
    assert!(result.is_null());
}
