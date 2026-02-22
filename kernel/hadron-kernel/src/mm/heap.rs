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

// ---------------------------------------------------------------------------
// Heap poisoning constants and types (always defined for cfg!() type-checking)
// ---------------------------------------------------------------------------

/// Heap poison fill values and red zone metadata. Defined unconditionally so
/// `cfg!()` branches type-check; optimized away when `hadron_debug_heap_poison`
/// is off.
mod poison {
    /// Uninitialized heap memory marker.
    pub const ALLOC_FILL: u8 = 0xCD;
    /// Freed heap memory marker.
    pub const FREE_FILL: u8 = 0xDD;
    /// Red zone fill byte (buffer overflow sentinel).
    pub const REDZONE_FILL: u8 = 0xFD;
    /// Red zone size in bytes (before + after each allocation).
    pub const REDZONE_SIZE: usize = 16;
    /// Magic value stored in the red zone header for integrity verification.
    pub const REDZONE_MAGIC: u32 = 0xFDFD_FDFD;

    /// Header stored in the last 16 bytes of the front red zone.
    ///
    /// Layout: `[alloc_size: u64, magic: u32, pad: [u8; 4]]` = 16 bytes.
    #[repr(C)]
    pub struct RedZoneHeader {
        /// The user-visible allocation size (after `max(MIN_BLOCK_SIZE)`).
        pub alloc_size: usize,
        /// Magic value (`REDZONE_MAGIC`) for corruption detection.
        pub magic: u32,
        /// Padding filled with `REDZONE_FILL`.
        pub _pad: [u8; 4],
    }

    /// Fills red zones and alloc pattern after a successful allocation.
    /// Returns the user-visible pointer (past the front red zone).
    ///
    /// # Safety
    ///
    /// `block_addr` must point to a valid, writable region of at least
    /// `front_pad + user_size + REDZONE_SIZE` bytes.
    pub unsafe fn fill_alloc(block_addr: usize, user_size: usize, front_pad: usize) -> *mut u8 {
        let user_ptr = block_addr + front_pad;

        // Fill front padding before the header with REDZONE_FILL.
        if front_pad > REDZONE_SIZE {
            // SAFETY: Region [block_addr..block_addr+front_pad-REDZONE_SIZE] is within
            // the allocated block.
            unsafe {
                core::ptr::write_bytes(block_addr as *mut u8, REDZONE_FILL, front_pad - REDZONE_SIZE);
            }
        }

        // Write header (last 16 bytes of front zone, immediately before user data).
        let header_ptr = (user_ptr - REDZONE_SIZE) as *mut RedZoneHeader;
        // SAFETY: header_ptr is within the allocated block and properly aligned
        // (REDZONE_SIZE = 16, block is 16-aligned).
        unsafe {
            (*header_ptr).alloc_size = user_size;
            (*header_ptr).magic = REDZONE_MAGIC;
            (*header_ptr)._pad = [REDZONE_FILL; 4];
        }

        // Fill user region with ALLOC_FILL (0xCD = "clean" / uninitialized).
        // SAFETY: user region is within the allocated block.
        unsafe {
            core::ptr::write_bytes(user_ptr as *mut u8, ALLOC_FILL, user_size);
        }

        // Fill back red zone with REDZONE_FILL.
        // SAFETY: back red zone is within the allocated block.
        unsafe {
            core::ptr::write_bytes((user_ptr + user_size) as *mut u8, REDZONE_FILL, REDZONE_SIZE);
        }

        user_ptr as *mut u8
    }

    /// Checks red zone integrity and fills freed region with `FREE_FILL`.
    ///
    /// # Panics
    ///
    /// Panics if any red zone byte has been corrupted (buffer overflow/underflow).
    ///
    /// # Safety
    ///
    /// `user_ptr` must be a pointer previously returned by [`fill_alloc`] with
    /// matching `user_size` and `front_pad`.
    pub unsafe fn check_and_fill_dealloc(user_ptr: *mut u8, user_size: usize, front_pad: usize) {
        let user_addr = user_ptr as usize;
        let block_addr = user_addr - front_pad;

        // Check header magic and size.
        let header_ptr = (user_addr - REDZONE_SIZE) as *const RedZoneHeader;
        // SAFETY: header_ptr was written by fill_alloc and is still valid.
        let header = unsafe { &*header_ptr };

        if header.magic != REDZONE_MAGIC {
            panic!(
                "heap corruption: front red zone magic mismatch at {:#x} (expected {:#x}, got {:#x})",
                user_addr, REDZONE_MAGIC, header.magic
            );
        }

        if header.alloc_size != user_size {
            panic!(
                "heap corruption: size mismatch at {:#x} (header says {}, layout says {})",
                user_addr, header.alloc_size, user_size
            );
        }

        // Check header padding bytes.
        if header._pad != [REDZONE_FILL; 4] {
            panic!(
                "heap corruption: front red zone padding corrupted at {:#x}",
                user_addr
            );
        }

        // Check front fill (before header, if front_pad > REDZONE_SIZE).
        if front_pad > REDZONE_SIZE {
            let front_fill = unsafe {
                core::slice::from_raw_parts(block_addr as *const u8, front_pad - REDZONE_SIZE)
            };
            for (i, &byte) in front_fill.iter().enumerate() {
                if byte != REDZONE_FILL {
                    panic!(
                        "heap corruption: front red zone byte {} corrupted at {:#x} \
                         (expected {:#x}, got {:#x})",
                        i,
                        block_addr + i,
                        REDZONE_FILL,
                        byte
                    );
                }
            }
        }

        // Check back red zone (16 bytes after user data).
        let back_start = user_addr + user_size;
        // SAFETY: back red zone was written by fill_alloc and is within the block.
        let back_zone = unsafe {
            core::slice::from_raw_parts(back_start as *const u8, REDZONE_SIZE)
        };
        for (i, &byte) in back_zone.iter().enumerate() {
            if byte != REDZONE_FILL {
                panic!(
                    "heap corruption: back red zone byte {} corrupted at {:#x} \
                     (expected {:#x}, got {:#x})",
                    i,
                    back_start + i,
                    REDZONE_FILL,
                    byte
                );
            }
        }

        // Fill user region with FREE_FILL (0xDD = "dead" / freed).
        // SAFETY: user region is within the block and we own it.
        unsafe {
            core::ptr::write_bytes(user_ptr, FREE_FILL, user_size);
        }
    }
}

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
    inner: SpinLock<LinkedListAllocatorInner>, // Lock level 0
}

// SAFETY: Protected by SpinLock.
unsafe impl Send for LinkedListAllocator {}
unsafe impl Sync for LinkedListAllocator {}

impl LinkedListAllocator {
    /// Creates a new, uninitialized allocator. Must call `init()` before use.
    pub const fn new() -> Self {
        Self {
            inner: SpinLock::leveled("HEAP", 1, LinkedListAllocatorInner {
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
        let mut inner = self.inner.lock_unchecked();
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
        let mut inner = self.inner.lock_unchecked();
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
        let user_size = layout.size().max(MIN_BLOCK_SIZE);
        let align = layout.align().max(BLOCK_ALIGN);

        // When heap poisoning is enabled, allocate extra space for red zones.
        // front_pad is aligned up to guarantee the user pointer stays aligned.
        let (alloc_size, front_pad) = if cfg!(hadron_debug_heap_poison) {
            let fp = align_up(poison::REDZONE_SIZE, align);
            (fp + user_size + poison::REDZONE_SIZE, fp)
        } else {
            (user_size, 0)
        };

        let mut inner = self.inner.lock_unchecked();

        // Try allocation.
        if let Some((addr, size)) = Self::find_first_fit(&mut inner, alloc_size, align) {
            inner.allocated_bytes += size;
            if cfg!(hadron_debug_alloc_track) {
                drop(inner);
                track_alloc(user_size);
            }
            if cfg!(hadron_debug_heap_poison) {
                // SAFETY: addr points to a valid region of at least alloc_size bytes.
                return unsafe { poison::fill_alloc(addr, user_size, front_pad) };
            }
            return addr as *mut u8;
        }

        // Try growing the heap.
        if let Some(grow) = inner.grow_fn {
            // Request at least the needed size, rounded up.
            let min_grow = alloc_size.max(64 * 1024); // at least 64 KiB
            drop(inner); // Release lock before calling grow_fn (it may need the PMM lock).

            if let Some((ptr, actual_size)) = grow(min_grow) {
                let mut inner = self.inner.lock_unchecked();
                unsafe {
                    Self::add_free_region(&mut inner, ptr as usize, actual_size);
                }
                inner.heap_end = inner.heap_end.max(ptr as usize + actual_size);

                if let Some((addr, size)) = Self::find_first_fit(&mut inner, alloc_size, align) {
                    inner.allocated_bytes += size;
                    if cfg!(hadron_debug_alloc_track) {
                        drop(inner);
                        track_alloc(user_size);
                    }
                    if cfg!(hadron_debug_heap_poison) {
                        // SAFETY: addr points to a valid region of at least alloc_size bytes.
                        return unsafe { poison::fill_alloc(addr, user_size, front_pad) };
                    }
                    return addr as *mut u8;
                }
            }
        }

        ptr::null_mut()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let user_size = layout.size().max(MIN_BLOCK_SIZE);
        let align = layout.align().max(BLOCK_ALIGN);

        // When heap poisoning is enabled, recover the full block (including red zones).
        let (block_addr, block_size) = if cfg!(hadron_debug_heap_poison) {
            let front_pad = align_up(poison::REDZONE_SIZE, align);
            // SAFETY: ptr was returned by our alloc with the same layout.
            unsafe { poison::check_and_fill_dealloc(ptr, user_size, front_pad) };
            let base = ptr as usize - front_pad;
            (base, front_pad + user_size + poison::REDZONE_SIZE)
        } else {
            (ptr as usize, user_size)
        };

        if cfg!(hadron_debug_alloc_track) {
            track_dealloc(user_size);
        }

        let mut inner = self.inner.lock_unchecked();
        inner.allocated_bytes -= block_size;

        unsafe {
            Self::add_free_region(&mut inner, block_addr, block_size);
        }
    }
}

#[inline]
fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

#[cfg_attr(target_os = "none", global_allocator)]
static HEAP: LinkedListAllocator = LinkedListAllocator::new();

/// Initializes the global heap allocator with a pre-mapped memory region.
///
/// # Safety
///
/// `heap_start` must point to a mapped, zeroed region of `heap_size` bytes.
pub unsafe fn init_raw(heap_start: usize, heap_size: usize) {
    unsafe { HEAP.init(heap_start, heap_size) };
}

/// Registers a growth callback for the global heap allocator.
pub fn register_grow_fn(f: fn(usize) -> Option<(*mut u8, usize)>) {
    HEAP.register_grow_fn(f);
}

/// Checks red zone integrity of all live heap allocations.
///
/// Red zones are checked on every `dealloc`; this function provides an
/// additional manual or periodic full-heap scan. A complete walk of live
/// allocations requires the allocation tracking side table
/// (`debug_alloc_track`); without it, only dealloc-time checks are active.
#[cfg(hadron_debug_heap_poison)]
pub fn scan_red_zones() {
    crate::kinfo!(
        "heap: red zone dealloc-time checks active; \
         full heap scan requires alloc tracking side table"
    );
}

// ---------------------------------------------------------------------------
// Allocation tracking (requires debug_alloc_track + debug_heap_poison)
// ---------------------------------------------------------------------------

/// Heap allocation statistics for leak investigation.
///
/// Defined unconditionally for `cfg!()` type-checking; compiled away when
/// `hadron_debug_alloc_track` is off.
struct AllocStats {
    /// Total number of allocations since boot.
    total_allocs: u64,
    /// Total number of frees since boot.
    total_frees: u64,
    /// Currently live allocations.
    current_live: u64,
    /// Currently allocated bytes (user-visible size, excludes red zones).
    current_bytes: u64,
    /// Peak live allocations.
    peak_live: u64,
    /// Peak allocated bytes.
    peak_bytes: u64,
}

impl AllocStats {
    const fn new() -> Self {
        Self {
            total_allocs: 0,
            total_frees: 0,
            current_live: 0,
            current_bytes: 0,
            peak_live: 0,
            peak_bytes: 0,
        }
    }

    fn record_alloc(&mut self, user_size: usize) {
        self.total_allocs += 1;
        self.current_live += 1;
        self.current_bytes += user_size as u64;
        if self.current_live > self.peak_live {
            self.peak_live = self.current_live;
        }
        if self.current_bytes > self.peak_bytes {
            self.peak_bytes = self.current_bytes;
        }
    }

    fn record_free(&mut self, user_size: usize) {
        self.total_frees += 1;
        self.current_live -= 1;
        self.current_bytes -= user_size as u64;
    }
}

static ALLOC_STATS: SpinLock<AllocStats> = SpinLock::named("ALLOC_STATS", AllocStats::new());

/// Records a successful allocation in the tracking stats.
///
/// Called from the `GlobalAlloc` impl when `debug_alloc_track` is enabled.
fn track_alloc(user_size: usize) {
    ALLOC_STATS.lock_unchecked().record_alloc(user_size);
}

/// Records a deallocation in the tracking stats.
///
/// Called from the `GlobalAlloc` impl when `debug_alloc_track` is enabled.
fn track_dealloc(user_size: usize) {
    ALLOC_STATS.lock_unchecked().record_free(user_size);
}

/// Logs current heap allocation statistics.
///
/// Reports total allocs/frees, current live count/bytes, and peak values.
/// Useful for leak investigation.
#[cfg(hadron_debug_alloc_track)]
pub fn dump_alloc_stats() {
    let stats = ALLOC_STATS.lock_unchecked();
    crate::kinfo!(
        "heap stats: allocs={} frees={} live={} bytes={} peak_live={} peak_bytes={}",
        stats.total_allocs,
        stats.total_frees,
        stats.current_live,
        stats.current_bytes,
        stats.peak_live,
        stats.peak_bytes
    );
}

// ---------------------------------------------------------------------------
// Kernel-level heap initialization
// ---------------------------------------------------------------------------

/// Initializes the kernel heap.
///
/// 1. Maps initial heap pages via VMM/PMM.
/// 2. Initializes the global linked-list allocator.
/// 3. Registers the growth callback.
pub fn init() {
    let (heap_start, heap_size) = super::vmm::map_initial_heap();

    unsafe {
        init_raw(heap_start, heap_size);
    }

    register_grow_fn(grow_callback);

    crate::ktrace_subsys!(
        mm,
        "heap initialized at {:#x}, size {:#x} ({} KiB)",
        heap_start,
        heap_size,
        heap_size / 1024
    );
}

/// Growth callback invoked by the heap allocator when it runs out of space.
fn grow_callback(min_bytes: usize) -> Option<(*mut u8, usize)> {
    super::vmm::grow_heap(min_bytes)
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

    // -----------------------------------------------------------------------
    // Poison module tests
    // -----------------------------------------------------------------------

    /// Allocates a buffer for poison testing with the given user_size and align.
    /// Returns (raw_buf, front_pad, total_size).
    fn alloc_poison_buf(user_size: usize, align: usize) -> (*mut u8, usize, usize) {
        let front_pad = align_up(poison::REDZONE_SIZE, align);
        let total = front_pad + user_size + poison::REDZONE_SIZE;
        let layout = Layout::from_size_align(total, align).unwrap();
        let buf = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!buf.is_null());
        (buf, front_pad, total)
    }

    /// Frees a poison test buffer.
    unsafe fn free_poison_buf(buf: *mut u8, total: usize, align: usize) {
        let layout = Layout::from_size_align(total, align).unwrap();
        unsafe { std::alloc::dealloc(buf, layout) };
    }

    #[test]
    fn test_redzone_header_size() {
        assert_eq!(
            core::mem::size_of::<poison::RedZoneHeader>(),
            poison::REDZONE_SIZE,
            "RedZoneHeader must be exactly REDZONE_SIZE bytes"
        );
    }

    #[test]
    fn test_fill_alloc_patterns_align16() {
        let user_size = 64;
        let align = 16;
        let (buf, front_pad, total) = alloc_poison_buf(user_size, align);

        // front_pad should be 16 (align_up(16, 16) = 16).
        assert_eq!(front_pad, 16);

        let user_ptr = unsafe { poison::fill_alloc(buf as usize, user_size, front_pad) };
        assert_eq!(user_ptr as usize, buf as usize + front_pad);

        // Check header at [buf..buf+16].
        let header = unsafe { &*((user_ptr as usize - poison::REDZONE_SIZE) as *const poison::RedZoneHeader) };
        assert_eq!(header.magic, poison::REDZONE_MAGIC);
        assert_eq!(header.alloc_size, user_size);
        assert_eq!(header._pad, [poison::REDZONE_FILL; 4]);

        // User region should be all ALLOC_FILL.
        let user_slice = unsafe { core::slice::from_raw_parts(user_ptr, user_size) };
        assert!(user_slice.iter().all(|&b| b == poison::ALLOC_FILL));

        // Back redzone should be all REDZONE_FILL.
        let back = unsafe { core::slice::from_raw_parts(user_ptr.add(user_size), poison::REDZONE_SIZE) };
        assert!(back.iter().all(|&b| b == poison::REDZONE_FILL));

        unsafe { free_poison_buf(buf, total, align) };
    }

    #[test]
    fn test_fill_alloc_patterns_align256() {
        let user_size = 64;
        let align = 256;
        let (buf, front_pad, total) = alloc_poison_buf(user_size, align);

        // front_pad should be 256 (align_up(16, 256) = 256).
        assert_eq!(front_pad, 256);

        let user_ptr = unsafe { poison::fill_alloc(buf as usize, user_size, front_pad) };
        assert_eq!(user_ptr as usize, buf as usize + front_pad);

        // Extended front fill [0..240] should be all REDZONE_FILL.
        let front_fill_len = front_pad - poison::REDZONE_SIZE; // 240
        let front_fill = unsafe { core::slice::from_raw_parts(buf, front_fill_len) };
        assert!(front_fill.iter().all(|&b| b == poison::REDZONE_FILL));

        // Header at [240..256].
        let header = unsafe { &*((user_ptr as usize - poison::REDZONE_SIZE) as *const poison::RedZoneHeader) };
        assert_eq!(header.magic, poison::REDZONE_MAGIC);
        assert_eq!(header.alloc_size, user_size);

        // User and back zones.
        let user_slice = unsafe { core::slice::from_raw_parts(user_ptr, user_size) };
        assert!(user_slice.iter().all(|&b| b == poison::ALLOC_FILL));
        let back = unsafe { core::slice::from_raw_parts(user_ptr.add(user_size), poison::REDZONE_SIZE) };
        assert!(back.iter().all(|&b| b == poison::REDZONE_FILL));

        unsafe { free_poison_buf(buf, total, align) };
    }

    #[test]
    fn test_check_dealloc_clean_roundtrip() {
        let user_size = 64;
        let align = 16;
        let (buf, front_pad, total) = alloc_poison_buf(user_size, align);

        let user_ptr = unsafe { poison::fill_alloc(buf as usize, user_size, front_pad) };
        // Should not panic — all zones are intact.
        unsafe { poison::check_and_fill_dealloc(user_ptr, user_size, front_pad) };

        // User region should now be all FREE_FILL.
        let user_slice = unsafe { core::slice::from_raw_parts(user_ptr, user_size) };
        assert!(user_slice.iter().all(|&b| b == poison::FREE_FILL));

        unsafe { free_poison_buf(buf, total, align) };
    }

    #[test]
    #[should_panic(expected = "front red zone magic mismatch")]
    fn test_check_dealloc_detects_magic_corruption() {
        let user_size = 64;
        let align = 16;
        let (buf, front_pad, total) = alloc_poison_buf(user_size, align);

        let user_ptr = unsafe { poison::fill_alloc(buf as usize, user_size, front_pad) };

        // Corrupt the magic field in the header.
        let header = (user_ptr as usize - poison::REDZONE_SIZE) as *mut poison::RedZoneHeader;
        unsafe { (*header).magic = 0xDEADBEEF };

        // Should panic.
        unsafe { poison::check_and_fill_dealloc(user_ptr, user_size, front_pad) };

        unsafe { free_poison_buf(buf, total, align) };
    }

    #[test]
    #[should_panic(expected = "back red zone byte")]
    fn test_check_dealloc_detects_back_corruption() {
        let user_size = 64;
        let align = 16;
        let (buf, front_pad, total) = alloc_poison_buf(user_size, align);

        let user_ptr = unsafe { poison::fill_alloc(buf as usize, user_size, front_pad) };

        // Write one byte past the user region (into back redzone).
        unsafe { *user_ptr.add(user_size) = 0x42 };

        // Should panic.
        unsafe { poison::check_and_fill_dealloc(user_ptr, user_size, front_pad) };

        unsafe { free_poison_buf(buf, total, align) };
    }

    #[test]
    #[should_panic(expected = "front red zone byte")]
    fn test_check_dealloc_detects_front_pad_corruption() {
        let user_size = 64;
        let align = 256;
        let (buf, front_pad, total) = alloc_poison_buf(user_size, align);

        let user_ptr = unsafe { poison::fill_alloc(buf as usize, user_size, front_pad) };

        // Corrupt a byte in the extended front fill region.
        unsafe { *buf.add(10) = 0x42 };

        // Should panic.
        unsafe { poison::check_and_fill_dealloc(user_ptr, user_size, front_pad) };

        unsafe { free_poison_buf(buf, total, align) };
    }

    #[test]
    #[should_panic(expected = "front red zone padding corrupted")]
    fn test_check_dealloc_detects_padding_corruption() {
        let user_size = 64;
        let align = 16;
        let (buf, front_pad, total) = alloc_poison_buf(user_size, align);

        let user_ptr = unsafe { poison::fill_alloc(buf as usize, user_size, front_pad) };

        // Corrupt the _pad field in the header.
        let header = (user_ptr as usize - poison::REDZONE_SIZE) as *mut poison::RedZoneHeader;
        unsafe { (*header)._pad[0] = 0x42 };

        // Should panic.
        unsafe { poison::check_and_fill_dealloc(user_ptr, user_size, front_pad) };

        unsafe { free_poison_buf(buf, total, align) };
    }

    // -----------------------------------------------------------------------
    // AllocStats tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_alloc_stats_initial() {
        let stats = AllocStats::new();
        assert_eq!(stats.total_allocs, 0);
        assert_eq!(stats.total_frees, 0);
        assert_eq!(stats.current_live, 0);
        assert_eq!(stats.current_bytes, 0);
        assert_eq!(stats.peak_live, 0);
        assert_eq!(stats.peak_bytes, 0);
    }

    #[test]
    fn test_alloc_stats_record_alloc() {
        let mut stats = AllocStats::new();
        stats.record_alloc(100);
        assert_eq!(stats.total_allocs, 1);
        assert_eq!(stats.current_live, 1);
        assert_eq!(stats.current_bytes, 100);
        assert_eq!(stats.peak_live, 1);
        assert_eq!(stats.peak_bytes, 100);
    }

    #[test]
    fn test_alloc_stats_record_free() {
        let mut stats = AllocStats::new();
        stats.record_alloc(100);
        stats.record_free(100);
        assert_eq!(stats.total_frees, 1);
        assert_eq!(stats.current_live, 0);
        assert_eq!(stats.current_bytes, 0);
        assert_eq!(stats.peak_live, 1);
        assert_eq!(stats.peak_bytes, 100);
    }

    #[test]
    fn test_alloc_stats_peak_tracking() {
        let mut stats = AllocStats::new();
        stats.record_alloc(64);   // live=1, bytes=64
        stats.record_alloc(128);  // live=2, bytes=192
        stats.record_free(64);    // live=1, bytes=128
        stats.record_alloc(256);  // live=2, bytes=384
        assert_eq!(stats.peak_live, 2);
        assert_eq!(stats.peak_bytes, 384);
        assert_eq!(stats.current_live, 2);
        assert_eq!(stats.current_bytes, 384);
    }

    #[test]
    fn test_alloc_stats_many_cycles() {
        let mut stats = AllocStats::new();
        for _ in 0..100 {
            stats.record_alloc(64);
            stats.record_free(64);
        }
        assert_eq!(stats.total_allocs, 100);
        assert_eq!(stats.total_frees, 100);
        assert_eq!(stats.current_live, 0);
        assert_eq!(stats.current_bytes, 0);
        assert_eq!(stats.peak_live, 1);
        assert_eq!(stats.peak_bytes, 64);
    }
}
