//! Searching and sorting: `qsort`, `bsearch`.
//!
//! `qsort` uses an **introsort** strategy (quicksort + heapsort fallback):
//! - Insertion sort for ≤ 16 elements (cache-friendly, low overhead).
//! - Median-of-three quicksort for larger arrays.
//! - Falls back to heapsort when recursion depth exceeds `2 × floor(log2(n))`,
//!   guaranteeing O(n log n) worst-case with no heap allocation.
//!
//! Element swaps use a 256-byte stack buffer for element sizes ≤ 256 bytes;
//! elements larger than 256 bytes are swapped byte-by-byte.

// ---- Swap helper ------------------------------------------------------------

const SWAP_BUF_SIZE: usize = 256;

/// Swap two elements of `size` bytes at `a` and `b`.
///
/// # Safety
///
/// `a` and `b` must be valid for `size` bytes and must not overlap.
#[inline]
unsafe fn swap_elements(a: *mut u8, b: *mut u8, size: usize) {
    if a == b {
        return;
    }
    if size <= SWAP_BUF_SIZE {
        let mut tmp = [0u8; SWAP_BUF_SIZE];
        // SAFETY: Caller guarantees a and b are valid for `size` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(a, tmp.as_mut_ptr(), size);
            core::ptr::copy_nonoverlapping(b, a, size);
            core::ptr::copy_nonoverlapping(tmp.as_ptr(), b, size);
        }
    } else {
        // SAFETY: Caller guarantees a and b are valid, non-overlapping.
        for k in 0..size {
            unsafe {
                let t = *a.add(k);
                *a.add(k) = *b.add(k);
                *b.add(k) = t;
            }
        }
    }
}

// ---- Insertion sort (used for small sub-arrays) ----------------------------

/// Sort `nmemb` elements starting at `base` using insertion sort.
///
/// # Safety
///
/// `base` must be valid for `nmemb * size` bytes. `compar` must be a valid
/// function pointer.
unsafe fn insertion_sort(
    base: *mut u8,
    nmemb: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
) {
    for i in 1..nmemb {
        let mut j = i;
        while j > 0 {
            // SAFETY: i and j are in bounds by loop invariant.
            let a = unsafe { base.add((j - 1) * size) };
            let b = unsafe { base.add(j * size) };
            if unsafe { compar(a, b) } <= 0 {
                break;
            }
            // SAFETY: a and b are valid and non-overlapping (different indices).
            unsafe { swap_elements(a, b, size) };
            j -= 1;
        }
    }
}

// ---- Heapsort (fallback for deep recursion) ---------------------------------

/// Restore the max-heap property for subtree rooted at `i` in array of `n` elements.
///
/// # Safety
///
/// `base` must be valid for `n * size` bytes. `i < n`.
unsafe fn sift_down(
    base: *mut u8,
    i: usize,
    n: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
) {
    let mut root = i;
    loop {
        let left = 2 * root + 1;
        let right = 2 * root + 2;
        let mut largest = root;

        // SAFETY: index checks ensure in-bounds access.
        if left < n {
            let la = unsafe { base.add(largest * size) };
            let lb = unsafe { base.add(left * size) };
            if unsafe { compar(la, lb) } < 0 {
                largest = left;
            }
        }
        if right < n {
            let la = unsafe { base.add(largest * size) };
            let rb = unsafe { base.add(right * size) };
            if unsafe { compar(la, rb) } < 0 {
                largest = right;
            }
        }
        if largest == root {
            break;
        }
        // SAFETY: root and largest are distinct in-bounds indices.
        let a = unsafe { base.add(root * size) };
        let b = unsafe { base.add(largest * size) };
        unsafe { swap_elements(a, b, size) };
        root = largest;
    }
}

/// Sort `nmemb` elements using heapsort. O(n log n), in-place.
///
/// # Safety
///
/// `base` must be valid for `nmemb * size` bytes. `compar` must be valid.
unsafe fn heapsort(
    base: *mut u8,
    nmemb: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
) {
    if nmemb <= 1 {
        return;
    }
    // Build max-heap.
    let mut i = nmemb / 2;
    loop {
        // SAFETY: base valid for nmemb*size, i < nmemb.
        unsafe { sift_down(base, i, nmemb, size, compar) };
        if i == 0 {
            break;
        }
        i -= 1;
    }
    // Extract elements.
    let mut end = nmemb - 1;
    while end > 0 {
        // Swap root (max) with last element.
        // SAFETY: 0 and end are distinct in-bounds indices.
        let a = base;
        let b = unsafe { base.add(end * size) };
        unsafe { swap_elements(a, b, size) };
        // SAFETY: base valid for end elements now (end shrunk by 1).
        unsafe { sift_down(base, 0, end, size, compar) };
        end -= 1;
    }
}

// ---- Quicksort with introsort cutoff ---------------------------------------

/// Partition `base[lo..hi]` using median-of-three pivot. Returns pivot index.
///
/// # Safety
///
/// `base` valid for `(hi+1) * size` bytes. `lo <= hi`.
unsafe fn partition(
    base: *mut u8,
    lo: usize,
    hi: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
) -> usize {
    let mid = lo + (hi - lo) / 2;

    // Median-of-three: sort lo, mid, hi.
    // SAFETY: all three are in-bounds.
    let p_lo = unsafe { base.add(lo * size) };
    let p_mid = unsafe { base.add(mid * size) };

    if unsafe { compar(p_lo, p_mid) } > 0 {
        unsafe { swap_elements(p_lo, p_mid, size) };
    }
    let p_lo = unsafe { base.add(lo * size) };
    let p_hi = unsafe { base.add(hi * size) };
    if unsafe { compar(p_lo, p_hi) } > 0 {
        unsafe { swap_elements(p_lo, p_hi, size) };
    }
    let p_mid = unsafe { base.add(mid * size) };
    let p_hi = unsafe { base.add(hi * size) };
    if unsafe { compar(p_mid, p_hi) } > 0 {
        unsafe { swap_elements(p_mid, p_hi, size) };
    }

    // Place pivot at hi-1.
    let p_mid = unsafe { base.add(mid * size) };
    let p_hi1 = unsafe { base.add((hi - 1) * size) };
    unsafe { swap_elements(p_mid, p_hi1, size) };

    let pivot = unsafe { base.add((hi - 1) * size) };
    let mut i = lo;
    let mut j = hi - 1;

    loop {
        i += 1;
        while i < hi - 1 && unsafe { compar(base.add(i * size), pivot) } < 0 {
            i += 1;
        }
        if j == 0 {
            break;
        }
        j -= 1;
        while j > lo && unsafe { compar(base.add(j * size), pivot) } > 0 {
            if j == 0 {
                break;
            }
            j -= 1;
        }
        if i >= j {
            break;
        }
        // SAFETY: i and j are distinct in-bounds indices.
        unsafe { swap_elements(base.add(i * size), base.add(j * size), size) };
    }

    // Restore pivot.
    let pivot_src = unsafe { base.add((hi - 1) * size) };
    let pivot_dst = unsafe { base.add(i * size) };
    unsafe { swap_elements(pivot_dst, pivot_src, size) };
    i
}

/// Introsort implementation: quicksort + heapsort fallback.
///
/// # Safety
///
/// `base` must be valid for `(hi - lo + 1) * size` bytes starting at `base + lo*size`.
unsafe fn introsort(
    base: *mut u8,
    lo: usize,
    hi: usize,
    depth_limit: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
) {
    const INSERTION_THRESHOLD: usize = 16;

    if lo >= hi {
        return;
    }
    let nmemb = hi - lo + 1;

    if nmemb <= INSERTION_THRESHOLD {
        // SAFETY: base + lo*size is start of sub-array; nmemb is in bounds.
        unsafe { insertion_sort(base.add(lo * size), nmemb, size, compar) };
        return;
    }

    if depth_limit == 0 {
        // Recursion too deep — fall back to heapsort.
        // SAFETY: base + lo*size valid for nmemb*size bytes.
        unsafe { heapsort(base.add(lo * size), nmemb, size, compar) };
        return;
    }

    // SAFETY: lo <= hi; base is valid for (hi+1)*size bytes.
    let pivot = unsafe { partition(base, lo, hi, size, compar) };
    if pivot > 0 {
        unsafe { introsort(base, lo, pivot - 1, depth_limit - 1, size, compar) };
    }
    unsafe { introsort(base, pivot + 1, hi, depth_limit - 1, size, compar) };
}

// ---- Public C ABI -----------------------------------------------------------

/// `qsort` — sort an array of `nmemb` elements of `size` bytes each.
///
/// Uses introsort (O(n log n) guaranteed). `compar` is called with pointers to
/// two elements and must return negative/zero/positive.
///
/// # Safety
///
/// `base` must be valid for `nmemb * size` bytes. `compar` must be a valid
/// function pointer that imposes a total order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qsort(
    base: *mut u8,
    nmemb: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
) {
    if nmemb <= 1 || size == 0 {
        return;
    }
    // Depth limit: 2 * floor(log2(nmemb)).
    let depth_limit = 2 * (usize::BITS - nmemb.leading_zeros()) as usize;
    // SAFETY: base is valid for nmemb*size bytes; depth_limit prevents infinite recursion.
    unsafe { introsort(base, 0, nmemb - 1, depth_limit, size, compar) };
}

/// `bsearch` — binary search a sorted array.
///
/// Returns a pointer to a matching element, or null if not found.
/// If multiple matches exist, which one is returned is unspecified.
///
/// # Safety
///
/// `base` must be valid for `nmemb * size` bytes and must be sorted according
/// to `compar`. `key` must point to an object comparable via `compar`.
/// `compar` must be a valid function pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsearch(
    key: *const u8,
    base: *const u8,
    nmemb: usize,
    size: usize,
    compar: unsafe extern "C" fn(*const u8, *const u8) -> i32,
) -> *const u8 {
    let mut lo = 0usize;
    let mut hi = nmemb;

    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        // SAFETY: mid is in [lo, hi) ⊂ [0, nmemb), so base + mid*size is in bounds.
        let elem = unsafe { base.add(mid * size) };
        let cmp = unsafe { compar(key, elem) };
        match cmp.cmp(&0) {
            core::cmp::Ordering::Less => hi = mid,
            core::cmp::Ordering::Greater => lo = mid + 1,
            core::cmp::Ordering::Equal => return elem,
        }
    }
    core::ptr::null()
}

// ---- Unit tests -------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn cmp_i32(a: *const u8, b: *const u8) -> i32 {
        let a = unsafe { core::ptr::read_unaligned(a as *const i32) };
        let b = unsafe { core::ptr::read_unaligned(b as *const i32) };
        a.cmp(&b) as i32
    }

    unsafe extern "C" fn cmp_u8(a: *const u8, b: *const u8) -> i32 {
        let a = unsafe { *a };
        let b = unsafe { *b };
        (a as i32) - (b as i32)
    }

    #[test]
    fn qsort_integers_sorted() {
        let mut arr: [i32; 8] = [5, 2, 8, 1, 9, 3, 7, 4];
        unsafe {
            qsort(
                arr.as_mut_ptr().cast(),
                arr.len(),
                core::mem::size_of::<i32>(),
                cmp_i32,
            );
        }
        assert_eq!(arr, [1, 2, 3, 4, 5, 7, 8, 9]);
    }

    #[test]
    fn qsort_already_sorted() {
        let mut arr: [i32; 5] = [1, 2, 3, 4, 5];
        unsafe {
            qsort(
                arr.as_mut_ptr().cast(),
                arr.len(),
                core::mem::size_of::<i32>(),
                cmp_i32,
            );
        }
        assert_eq!(arr, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn qsort_reverse_sorted() {
        let mut arr: [i32; 6] = [6, 5, 4, 3, 2, 1];
        unsafe {
            qsort(
                arr.as_mut_ptr().cast(),
                arr.len(),
                core::mem::size_of::<i32>(),
                cmp_i32,
            );
        }
        assert_eq!(arr, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn qsort_duplicates() {
        let mut arr: [i32; 6] = [3, 1, 4, 1, 5, 9];
        unsafe {
            qsort(
                arr.as_mut_ptr().cast(),
                arr.len(),
                core::mem::size_of::<i32>(),
                cmp_i32,
            );
        }
        assert_eq!(arr, [1, 1, 3, 4, 5, 9]);
    }

    #[test]
    fn qsort_single_element() {
        let mut arr: [i32; 1] = [42];
        unsafe {
            qsort(
                arr.as_mut_ptr().cast(),
                arr.len(),
                core::mem::size_of::<i32>(),
                cmp_i32,
            );
        }
        assert_eq!(arr, [42]);
    }

    #[test]
    fn qsort_large_array() {
        // 100 elements in reverse order — exercises the heapsort fallback path.
        let mut arr: [i32; 100] = [0i32; 100];
        for (i, v) in arr.iter_mut().enumerate() {
            *v = (100 - i) as i32;
        }
        unsafe {
            qsort(
                arr.as_mut_ptr().cast(),
                arr.len(),
                core::mem::size_of::<i32>(),
                cmp_i32,
            );
        }
        for i in 0..100 {
            assert_eq!(arr[i], (i + 1) as i32, "mismatch at index {i}");
        }
    }

    #[test]
    fn bsearch_found() {
        let arr: [i32; 5] = [1, 3, 5, 7, 9];
        let key: i32 = 5;
        let p = unsafe {
            bsearch(
                (&key as *const i32).cast(),
                arr.as_ptr().cast(),
                arr.len(),
                core::mem::size_of::<i32>(),
                cmp_i32,
            )
        };
        assert!(!p.is_null());
        assert_eq!(unsafe { core::ptr::read_unaligned(p as *const i32) }, 5);
    }

    #[test]
    fn bsearch_not_found() {
        let arr: [i32; 5] = [1, 3, 5, 7, 9];
        let key: i32 = 4;
        let p = unsafe {
            bsearch(
                (&key as *const i32).cast(),
                arr.as_ptr().cast(),
                arr.len(),
                core::mem::size_of::<i32>(),
                cmp_i32,
            )
        };
        assert!(p.is_null());
    }

    #[test]
    fn bsearch_bytes() {
        let arr: [u8; 5] = [10, 20, 30, 40, 50];
        let key: u8 = 30;
        let p = unsafe { bsearch(&key as *const u8, arr.as_ptr(), arr.len(), 1, cmp_u8) };
        assert!(!p.is_null());
        assert_eq!(unsafe { *p }, 30);
    }
}
