//! Userspace heap allocator backed by `sys_mem_map`.
//!
//! A simple bump-with-freelist allocator implementing [`GlobalAlloc`]. Grows
//! by requesting 64 KiB chunks from the kernel via `sys_mem_map`. Freed
//! blocks are tracked in a single-linked free list for reuse.
//!
//! This allocator is single-threaded (Hadron userspace is currently
//! single-threaded per process), so no locking is needed.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;

/// Chunk size requested from the kernel when more memory is needed.
const CHUNK_SIZE: usize = 64 * 1024; // 64 KiB

/// Minimum alignment for all allocations (matches `size_of::<usize>() * 2`).
const MIN_ALIGN: usize = 16;

/// Header placed at the start of each free block in the free list.
struct FreeBlock {
    /// Size of this block (including header).
    size: usize,
    /// Pointer to the next free block, or null.
    next: *mut FreeBlock,
}

/// Bump-with-freelist allocator state.
struct HeapInner {
    /// Current bump pointer within the active chunk.
    bump: *mut u8,
    /// End of the active chunk (one past the last usable byte).
    bump_end: *mut u8,
    /// Head of the free list.
    free_list: *mut FreeBlock,
}

/// The global userspace heap allocator.
pub struct UserHeap {
    inner: UnsafeCell<HeapInner>,
}

// SAFETY: Hadron userspace processes are single-threaded. No concurrent access.
unsafe impl Sync for UserHeap {}

impl UserHeap {
    /// Creates a new heap allocator. No memory is allocated until the first use.
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(HeapInner {
                bump: core::ptr::null_mut(),
                bump_end: core::ptr::null_mut(),
                free_list: core::ptr::null_mut(),
            }),
        }
    }
}

/// Align `addr` upward to `align` (must be a power of two).
#[inline]
const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

/// Grow the heap by requesting a new chunk from the kernel.
///
/// Returns `true` on success. On success, `inner.bump` and `inner.bump_end`
/// are updated to point into the new chunk.
fn grow(inner: &mut HeapInner, min_size: usize) -> bool {
    let size = if min_size > CHUNK_SIZE {
        align_up(min_size, 4096) // Page-align large requests.
    } else {
        CHUNK_SIZE
    };

    match crate::sys::mem_map(size) {
        Some(ptr) => {
            inner.bump = ptr;
            // SAFETY: mem_map returned a valid region of `size` bytes.
            inner.bump_end = unsafe { ptr.add(size) };
            true
        }
        None => false,
    }
}

unsafe impl GlobalAlloc for UserHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Single-threaded; no concurrent access to inner.
        let inner = unsafe { &mut *self.inner.get() };
        let align = layout.align().max(MIN_ALIGN);
        let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), align);

        // 1. Try the free list: first-fit.
        let mut prev: *mut *mut FreeBlock = &mut inner.free_list;
        let mut current = inner.free_list;
        while !current.is_null() {
            // SAFETY: current is a valid FreeBlock pointer from our free list.
            let block = unsafe { &mut *current };
            let block_addr = current as usize;
            let aligned_addr = align_up(block_addr, align);
            let padding = aligned_addr - block_addr;

            if block.size >= size + padding {
                // Use this block. Remove from free list.
                // SAFETY: prev points to a valid *mut FreeBlock field.
                unsafe {
                    *prev = block.next;
                }
                return aligned_addr as *mut u8;
            }
            // SAFETY: prev tracks the link that points to current.
            prev = unsafe { &mut (*current).next };
            current = block.next;
        }

        // 2. Try bump allocation from the active chunk.
        let bump_addr = align_up(inner.bump as usize, align);
        let bump_end = bump_addr + size;
        if bump_end <= inner.bump_end as usize {
            inner.bump = bump_end as *mut u8;
            return bump_addr as *mut u8;
        }

        // 3. Need a new chunk.
        if !grow(inner, size + align) {
            return core::ptr::null_mut();
        }

        // Bump from the fresh chunk.
        let bump_addr = align_up(inner.bump as usize, align);
        let bump_end = bump_addr + size;
        inner.bump = bump_end as *mut u8;
        bump_addr as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Single-threaded; no concurrent access to inner.
        let inner = unsafe { &mut *self.inner.get() };
        let align = layout.align().max(MIN_ALIGN);
        let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), align);

        // Push onto free list.
        let block = ptr as *mut FreeBlock;
        // SAFETY: ptr was allocated by us with at least size_of::<FreeBlock>() bytes.
        unsafe {
            (*block).size = size;
            (*block).next = inner.free_list;
        }
        inner.free_list = block;
    }
}
