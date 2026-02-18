//! Linked-list heap allocator implementing `GlobalAlloc`.
//!
//! First-fit free list sorted by address for O(1) coalescing on dealloc.
//! Supports a growth callback so the heap can request more pages from the
//! VMM/PMM when it runs out.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use crate::sync::SpinLock;

/// Minimum block size (must fit a `FreeBlock` header).
const MIN_BLOCK_SIZE: usize = 32;

/// Align all blocks to at least 16 bytes for SSE compatibility.
const BLOCK_ALIGN: usize = 16;

/// Free block header, stored at the start of each free block.
#[repr(C)]
struct FreeBlock {
    /// Total size of this block including the header.
    size: usize,
    /// Pointer to the next free block (sorted by address), or null.
    next: *mut FreeBlock,
}

struct LinkedListAllocatorInner {
    /// Head of the free list (sorted by address).
    head: *mut FreeBlock,
    /// Start address of the heap region.
    heap_start: usize,
    /// End address of the heap region (grows as pages are added).
    heap_end: usize,
    /// Number of bytes currently allocated.
    allocated_bytes: usize,
    /// Callback to grow the heap: takes minimum bytes needed, returns
    /// `(ptr, actual_size)` of newly mapped region.
    grow_fn: Option<fn(usize) -> Option<(*mut u8, usize)>>,
}

// SAFETY: The inner state is only accessed under the SpinLock.
unsafe impl Send for LinkedListAllocatorInner {}

/// A linked-list heap allocator suitable for use as `#[global_allocator]`.
///
/// Uses first-fit allocation with address-sorted free list and immediate
/// coalescing on dealloc.
pub struct LinkedListAllocator {
    inner: SpinLock<LinkedListAllocatorInner>,
}

// SAFETY: Protected by SpinLock.
unsafe impl Send for LinkedListAllocator {}
unsafe impl Sync for LinkedListAllocator {}

impl LinkedListAllocator {
    /// Creates a new, uninitialized allocator. Must call `init()` before use.
    pub const fn new() -> Self {
        Self {
            inner: SpinLock::new(LinkedListAllocatorInner {
                head: ptr::null_mut(),
                heap_start: 0,
                heap_end: 0,
                allocated_bytes: 0,
                grow_fn: None,
            }),
        }
    }

    /// Initializes the allocator with the given heap region.
    ///
    /// # Safety
    ///
    /// - `heap_start` must be page-aligned and point to mapped, zeroed memory.
    /// - `heap_size` must be the exact size of the mapped region.
    /// - Must be called exactly once.
    pub unsafe fn init(&self, heap_start: usize, heap_size: usize) {
        let mut inner = self.inner.lock();
        debug_assert!(inner.head.is_null(), "heap already initialized");
        debug_assert!(heap_size >= MIN_BLOCK_SIZE, "heap too small");

        inner.heap_start = heap_start;
        inner.heap_end = heap_start + heap_size;

        // Create a single free block spanning the entire heap.
        let block = heap_start as *mut FreeBlock;
        unsafe {
            (*block).size = heap_size;
            (*block).next = ptr::null_mut();
        }
        inner.head = block;
    }

    /// Registers a callback that the allocator uses to request more heap pages.
    pub fn register_grow_fn(&self, f: fn(usize) -> Option<(*mut u8, usize)>) {
        let mut inner = self.inner.lock();
        inner.grow_fn = Some(f);
    }

    /// Adds a new free region to the heap.
    ///
    /// # Safety
    ///
    /// The region must be valid, mapped, and not overlap with existing heap regions.
    unsafe fn add_free_region(inner: &mut LinkedListAllocatorInner, addr: usize, size: usize) {
        debug_assert!(size >= MIN_BLOCK_SIZE);
        debug_assert!(addr % BLOCK_ALIGN == 0);

        let new_block = addr as *mut FreeBlock;
        unsafe {
            (*new_block).size = size;
            (*new_block).next = ptr::null_mut();
        }

        // Insert in sorted order and coalesce.
        Self::insert_and_coalesce(inner, new_block);
    }

    /// Inserts a block into the free list in address-sorted order,
    /// coalescing with adjacent blocks.
    fn insert_and_coalesce(inner: &mut LinkedListAllocatorInner, block: *mut FreeBlock) {
        let block_addr = block as usize;
        let block_size = unsafe { (*block).size };

        // Find insertion point.
        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut current = inner.head;

        while !current.is_null() && (current as usize) < block_addr {
            prev = current;
            current = unsafe { (*current).next };
        }

        // Try coalesce with predecessor.
        if !prev.is_null() {
            let prev_end = prev as usize + unsafe { (*prev).size };
            if prev_end == block_addr {
                // Merge block into prev.
                unsafe {
                    (*prev).size += block_size;
                }
                // Try coalesce merged block with successor.
                let merged_end = prev as usize + unsafe { (*prev).size };
                if !current.is_null() && merged_end == current as usize {
                    unsafe {
                        (*prev).size += (*current).size;
                        (*prev).next = (*current).next;
                    }
                }
                return;
            }
        }

        // Try coalesce with successor.
        if !current.is_null() {
            let block_end = block_addr + block_size;
            if block_end == current as usize {
                unsafe {
                    (*block).size += (*current).size;
                    (*block).next = (*current).next;
                }
            } else {
                unsafe {
                    (*block).next = current;
                }
            }
        } else {
            unsafe {
                (*block).next = ptr::null_mut();
            }
        }

        // Link from predecessor.
        if prev.is_null() {
            inner.head = block;
        } else {
            unsafe {
                (*prev).next = block;
            }
        }
    }

    /// Finds and removes a suitable block from the free list (first-fit).
    /// Returns `(block_start, block_size)` or `None`.
    fn find_first_fit(
        inner: &mut LinkedListAllocatorInner,
        size: usize,
        align: usize,
    ) -> Option<(usize, usize)> {
        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut current = inner.head;

        while !current.is_null() {
            let block_addr = current as usize;
            let block_size = unsafe { (*current).size };

            // Calculate aligned allocation start within this block.
            let alloc_start = align_up(block_addr, align);
            let alloc_end = alloc_start.checked_add(size)?;

            if alloc_end <= block_addr + block_size {
                // This block is large enough.
                let next = unsafe { (*current).next };

                // Unlink from free list.
                if prev.is_null() {
                    inner.head = next;
                } else {
                    unsafe {
                        (*prev).next = next;
                    }
                }

                // If there's space before the aligned start, return it as a free block.
                let front_padding = alloc_start - block_addr;
                if front_padding >= MIN_BLOCK_SIZE {
                    let front = block_addr as *mut FreeBlock;
                    unsafe {
                        (*front).size = front_padding;
                        (*front).next = ptr::null_mut();
                    }
                    Self::insert_and_coalesce(inner, front);
                }

                // If the remainder after allocation is large enough, split it off.
                let used_size = (alloc_start - block_addr) + size;
                let remainder = block_size - used_size;
                if remainder >= MIN_BLOCK_SIZE {
                    let rem_addr = alloc_start + size;
                    let rem_aligned = align_up(rem_addr, BLOCK_ALIGN);
                    let rem_size = block_addr + block_size - rem_aligned;
                    if rem_size >= MIN_BLOCK_SIZE {
                        unsafe {
                            Self::add_free_region(inner, rem_aligned, rem_size);
                        }
                    }
                }

                return Some((alloc_start, size));
            }

            prev = current;
            current = unsafe { (*current).next };
        }

        None
    }
}

unsafe impl GlobalAlloc for LinkedListAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(MIN_BLOCK_SIZE);
        let align = layout.align().max(BLOCK_ALIGN);

        let mut inner = self.inner.lock();

        // Try allocation.
        if let Some((addr, alloc_size)) = Self::find_first_fit(&mut inner, size, align) {
            inner.allocated_bytes += alloc_size;
            return addr as *mut u8;
        }

        // Try growing the heap.
        if let Some(grow) = inner.grow_fn {
            // Request at least the needed size, rounded up.
            let min_grow = size.max(64 * 1024); // at least 64 KiB
            drop(inner); // Release lock before calling grow_fn (it may need the PMM lock).

            if let Some((ptr, actual_size)) = grow(min_grow) {
                let mut inner = self.inner.lock();
                unsafe {
                    Self::add_free_region(&mut inner, ptr as usize, actual_size);
                }
                inner.heap_end = inner.heap_end.max(ptr as usize + actual_size);

                if let Some((addr, alloc_size)) = Self::find_first_fit(&mut inner, size, align) {
                    inner.allocated_bytes += alloc_size;
                    return addr as *mut u8;
                }
            }
        }

        ptr::null_mut()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(MIN_BLOCK_SIZE);
        let addr = ptr as usize;

        let mut inner = self.inner.lock();
        inner.allocated_bytes -= size;

        unsafe {
            Self::add_free_region(&mut inner, addr, size);
        }
    }
}

#[inline]
fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

#[cfg_attr(target_os = "none", global_allocator)]
static HEAP: LinkedListAllocator = LinkedListAllocator::new();

/// Initializes the global heap allocator.
///
/// # Safety
///
/// `heap_start` must point to a mapped, zeroed region of `heap_size` bytes.
pub unsafe fn init(heap_start: usize, heap_size: usize) {
    unsafe { HEAP.init(heap_start, heap_size) };
}

/// Registers a growth callback for the global heap allocator.
pub fn register_grow_fn(f: fn(usize) -> Option<(*mut u8, usize)>) {
    HEAP.register_grow_fn(f);
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    fn with_test_heap<F: FnOnce(&LinkedListAllocator)>(size: usize, f: F) {
        let layout = Layout::from_size_align(size, BLOCK_ALIGN).unwrap();
        let buf = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!buf.is_null());
        let alloc = LinkedListAllocator::new();
        unsafe { alloc.init(buf as usize, size) };
        f(&alloc);
        unsafe { std::alloc::dealloc(buf, layout) };
    }

    #[test]
    fn align_up_already_aligned() {
        assert_eq!(align_up(0x1000, 16), 0x1000);
    }

    #[test]
    fn align_up_rounds() {
        assert_eq!(align_up(0x1001, 16), 0x1010);
    }

    #[test]
    fn align_up_zero() {
        assert_eq!(align_up(0, 16), 0);
    }

    #[test]
    fn alloc_and_dealloc_single() {
        with_test_heap(4096, |alloc| {
            let layout = Layout::from_size_align(64, 16).unwrap();
            let ptr = unsafe { alloc.alloc(layout) };
            assert!(!ptr.is_null());
            unsafe { alloc.dealloc(ptr, layout) };
        });
    }

    #[test]
    fn alloc_respects_alignment() {
        with_test_heap(4096, |alloc| {
            let layout = Layout::from_size_align(64, 256).unwrap();
            let ptr = unsafe { alloc.alloc(layout) };
            assert!(!ptr.is_null());
            assert_eq!(ptr as usize % 256, 0, "pointer not aligned to 256");
            unsafe { alloc.dealloc(ptr, layout) };
        });
    }

    #[test]
    fn alloc_returns_null_when_exhausted() {
        with_test_heap(128, |alloc| {
            let layout = Layout::from_size_align(64, 16).unwrap();
            let p1 = unsafe { alloc.alloc(layout) };
            assert!(!p1.is_null());
            let p2 = unsafe { alloc.alloc(layout) };
            assert!(!p2.is_null());
            // Heap is 128 bytes, both 64-byte allocs used it all (with MIN_BLOCK_SIZE=32).
            let p3 = unsafe { alloc.alloc(layout) };
            assert!(p3.is_null());
            unsafe {
                alloc.dealloc(p1, layout);
                alloc.dealloc(p2, layout);
            }
        });
    }

    #[test]
    fn dealloc_coalesces_adjacent() {
        with_test_heap(4096, |alloc| {
            let layout_a = Layout::from_size_align(64, 16).unwrap();
            let layout_b = Layout::from_size_align(64, 16).unwrap();
            let layout_c = Layout::from_size_align(64, 16).unwrap();

            let a = unsafe { alloc.alloc(layout_a) };
            let b = unsafe { alloc.alloc(layout_b) };
            let c = unsafe { alloc.alloc(layout_c) };
            assert!(!a.is_null() && !b.is_null() && !c.is_null());

            // Dealloc B then A — should coalesce into one region.
            unsafe {
                alloc.dealloc(b, layout_b);
                alloc.dealloc(a, layout_a);
            }

            // Now allocate something that requires A+B space combined.
            let layout_ab = Layout::from_size_align(96, 16).unwrap();
            let ab = unsafe { alloc.alloc(layout_ab) };
            assert!(
                !ab.is_null(),
                "coalesced region should satisfy larger alloc"
            );

            unsafe {
                alloc.dealloc(ab, layout_ab);
                alloc.dealloc(c, layout_c);
            }
        });
    }

    #[test]
    fn dealloc_coalesces_with_successor() {
        with_test_heap(4096, |alloc| {
            let layout = Layout::from_size_align(64, 16).unwrap();
            let a = unsafe { alloc.alloc(layout) };
            let b = unsafe { alloc.alloc(layout) };
            assert!(!a.is_null() && !b.is_null());

            // Dealloc A then B — should coalesce.
            unsafe {
                alloc.dealloc(a, layout);
                alloc.dealloc(b, layout);
            }

            // Full heap should be available again.
            let big_layout = Layout::from_size_align(4000, 16).unwrap();
            let big = unsafe { alloc.alloc(big_layout) };
            assert!(
                !big.is_null(),
                "full region should be available after coalescing"
            );
            unsafe { alloc.dealloc(big, big_layout) };
        });
    }

    #[test]
    fn alloc_splits_large_block() {
        with_test_heap(4096, |alloc| {
            let small = Layout::from_size_align(64, 16).unwrap();
            let p1 = unsafe { alloc.alloc(small) };
            assert!(!p1.is_null());

            // Remainder should be usable.
            let medium = Layout::from_size_align(256, 16).unwrap();
            let p2 = unsafe { alloc.alloc(medium) };
            assert!(!p2.is_null());

            unsafe {
                alloc.dealloc(p1, small);
                alloc.dealloc(p2, medium);
            }
        });
    }

    #[test]
    fn min_block_size_enforced() {
        with_test_heap(4096, |alloc| {
            // Alloc 1 byte — should use MIN_BLOCK_SIZE (32) internally.
            let layout = Layout::from_size_align(1, 1).unwrap();
            let ptr = unsafe { alloc.alloc(layout) };
            assert!(!ptr.is_null());
            unsafe { alloc.dealloc(ptr, layout) };
        });
    }

    #[test]
    fn grow_callback_invoked() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static GROW_CALLED: AtomicBool = AtomicBool::new(false);

        fn grow_fn(_min_size: usize) -> Option<(*mut u8, usize)> {
            GROW_CALLED.store(true, Ordering::SeqCst);
            None // Return None — we just want to verify it was called.
        }

        GROW_CALLED.store(false, Ordering::SeqCst);

        // Use MIN_BLOCK_SIZE (32) heap so one alloc fills it entirely,
        // forcing the second alloc to invoke the grow callback.
        with_test_heap(32, |alloc| {
            alloc.register_grow_fn(grow_fn);

            // Fill the entire heap with a single allocation.
            let layout = Layout::from_size_align(16, 16).unwrap();
            let p1 = unsafe { alloc.alloc(layout) };
            assert!(!p1.is_null());

            // This should trigger the grow callback since heap is exhausted.
            let p2 = unsafe { alloc.alloc(layout) };
            // p2 will be null since grow returns None, but grow should have been called.
            assert!(
                GROW_CALLED.load(Ordering::SeqCst),
                "grow callback should have been invoked"
            );

            unsafe { alloc.dealloc(p1, layout) };
            if !p2.is_null() {
                unsafe { alloc.dealloc(p2, layout) };
            }
        });
    }

    #[test]
    fn multiple_alloc_dealloc_cycles() {
        with_test_heap(8192, |alloc| {
            let layout = Layout::from_size_align(64, 16).unwrap();
            for _ in 0..100 {
                let ptr = unsafe { alloc.alloc(layout) };
                assert!(!ptr.is_null(), "alloc should not fail in cycle");
                unsafe { alloc.dealloc(ptr, layout) };
            }
        });
    }
}
