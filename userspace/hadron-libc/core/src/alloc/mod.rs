//! C-compatible heap allocator for libc.
//!
//! Provides `malloc`, `calloc`, `realloc`, and `free` backed by a
//! bump-with-freelist allocator that requests memory from the kernel
//! via `sys_mmap` in 64 KiB chunks.
//!
//! Phase 1: single static instance, no locking (single-threaded).

mod mmap;

use core::cell::UnsafeCell;
use core::ptr;

/// Chunk size requested from the kernel when more memory is needed.
const CHUNK_SIZE: usize = 64 * 1024; // 64 KiB

/// Minimum alignment for all allocations (16-byte aligned).
const MIN_ALIGN: usize = 16;

/// Header prepended to every allocation for `realloc` and `free`.
#[repr(C)]
struct BlockHeader {
    /// Usable size of this block (excluding header).
    size: usize,
    /// Padding to maintain 16-byte alignment.
    _pad: usize,
}

const HEADER_SIZE: usize = core::mem::size_of::<BlockHeader>();

/// Free list node — reuses the block's user area.
struct FreeNode {
    /// Total block size (header + user area).
    total_size: usize,
    /// Next free node, or null.
    next: *mut FreeNode,
}

/// Bump-with-freelist allocator state.
struct AllocInner {
    bump: *mut u8,
    bump_end: *mut u8,
    free_list: *mut FreeNode,
}

struct Allocator(UnsafeCell<AllocInner>);

// SAFETY: Hadron userspace is single-threaded in Phase 1.
unsafe impl Sync for Allocator {}

static ALLOC: Allocator = Allocator(UnsafeCell::new(AllocInner {
    bump: ptr::null_mut(),
    bump_end: ptr::null_mut(),
    free_list: ptr::null_mut(),
}));

#[inline]
const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

/// Grow the heap by requesting a new chunk from the kernel.
fn grow(inner: &mut AllocInner, min_size: usize) -> bool {
    let size = if min_size > CHUNK_SIZE {
        align_up(min_size, 4096)
    } else {
        CHUNK_SIZE
    };

    match mmap::request_pages(size) {
        Some(ptr) => {
            inner.bump = ptr;
            // SAFETY: mmap returned a valid region of `size` bytes.
            inner.bump_end = unsafe { ptr.add(size) };
            true
        }
        None => false,
    }
}

fn alloc_inner(size: usize) -> *mut u8 {
    // SAFETY: Single-threaded; no concurrent access.
    let inner = unsafe { &mut *ALLOC.0.get() };
    let total = HEADER_SIZE + align_up(size.max(core::mem::size_of::<FreeNode>()), MIN_ALIGN);

    // 1. Try free list: first-fit.
    let mut prev: *mut *mut FreeNode = &mut inner.free_list;
    let mut current = inner.free_list;
    while !current.is_null() {
        // SAFETY: current is a valid FreeNode from our free list.
        let node = unsafe { &mut *current };
        if node.total_size >= total {
            // Remove from free list.
            // SAFETY: prev points to a valid link.
            unsafe { *prev = node.next };
            let header = current.cast::<BlockHeader>();
            // SAFETY: Writing to memory we own.
            unsafe { (*header).size = node.total_size - HEADER_SIZE };
            return unsafe { (header as *mut u8).add(HEADER_SIZE) };
        }
        // SAFETY: Advancing through valid free list links.
        prev = unsafe { &mut (*current).next };
        current = node.next;
    }

    // 2. Try bump allocation.
    let bump_addr = align_up(inner.bump as usize, MIN_ALIGN);
    let bump_end = bump_addr + total;
    if bump_end <= inner.bump_end as usize {
        inner.bump = bump_end as *mut u8;
        let header = bump_addr as *mut BlockHeader;
        // SAFETY: Writing to freshly bump-allocated memory.
        unsafe { (*header).size = total - HEADER_SIZE };
        return unsafe { (header as *mut u8).add(HEADER_SIZE) };
    }

    // 3. Need a new chunk.
    if !grow(inner, total + MIN_ALIGN) {
        return ptr::null_mut();
    }

    let bump_addr = align_up(inner.bump as usize, MIN_ALIGN);
    let bump_end = bump_addr + total;
    inner.bump = bump_end as *mut u8;
    let header = bump_addr as *mut BlockHeader;
    // SAFETY: Writing to freshly allocated memory from a new chunk.
    unsafe { (*header).size = total - HEADER_SIZE };
    unsafe { (header as *mut u8).add(HEADER_SIZE) }
}

/// `malloc(size)` — allocate `size` bytes of uninitialized memory.
///
/// Returns a pointer to the allocated block, or null on failure.
/// The returned pointer is 16-byte aligned.
///
/// # Safety
///
/// The returned pointer must be freed with [`free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut u8 {
    if size == 0 {
        return ptr::null_mut();
    }
    alloc_inner(size)
}

/// `calloc(nmemb, size)` — allocate zeroed memory for an array.
///
/// # Safety
///
/// The returned pointer must be freed with [`free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut u8 {
    let total = match nmemb.checked_mul(size) {
        Some(t) => t,
        None => return ptr::null_mut(),
    };
    if total == 0 {
        return ptr::null_mut();
    }
    let p = alloc_inner(total);
    if !p.is_null() {
        // SAFETY: p points to at least `total` bytes of valid memory.
        unsafe { ptr::write_bytes(p, 0, total) };
    }
    p
}

/// `realloc(ptr, size)` — resize a previously allocated block.
///
/// # Safety
///
/// `ptr` must be null or a pointer previously returned by [`malloc`]/[`calloc`]/[`realloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(ptr: *mut u8, size: usize) -> *mut u8 {
    if ptr.is_null() {
        return unsafe { malloc(size) };
    }
    if size == 0 {
        unsafe { free(ptr) };
        return ptr::null_mut();
    }

    // Read old size from header.
    // SAFETY: ptr was returned by alloc_inner, header is at ptr - HEADER_SIZE.
    let header = unsafe { (ptr as *mut BlockHeader).sub(1) };
    let old_size = unsafe { (*header).size };

    if size <= old_size {
        return ptr;
    }

    // Allocate new block and copy.
    let new_ptr = alloc_inner(size);
    if !new_ptr.is_null() {
        // SAFETY: Both pointers are valid for old_size bytes.
        unsafe { ptr::copy_nonoverlapping(ptr, new_ptr, old_size) };
        unsafe { free(ptr) };
    }
    new_ptr
}

/// `free(ptr)` — free a previously allocated block.
///
/// # Safety
///
/// `ptr` must be null or a pointer previously returned by [`malloc`]/[`calloc`]/[`realloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }

    // SAFETY: Single-threaded access.
    let inner = unsafe { &mut *ALLOC.0.get() };

    // Read header.
    // SAFETY: ptr was returned by alloc_inner, header is at ptr - HEADER_SIZE.
    let header = unsafe { (ptr as *mut BlockHeader).sub(1) };
    let total_size = unsafe { (*header).size } + HEADER_SIZE;

    // Push onto free list.
    let node = header.cast::<FreeNode>();
    // SAFETY: The block is at least size_of::<FreeNode>() bytes.
    unsafe {
        (*node).total_size = total_size;
        (*node).next = inner.free_list;
    }
    inner.free_list = node;
}
