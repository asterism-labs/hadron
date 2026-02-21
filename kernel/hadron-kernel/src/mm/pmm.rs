//! Bitmap-based physical frame allocator.
//!
//! Uses a bitmap stored in HHDM-accessible memory where each bit represents
//! one 4 KiB frame. Bit = 1 means allocated/reserved, bit = 0 means free.
//! Word-level scanning with `trailing_zeros()` (compiles to TZCNT/BSF on
//! x86_64) provides efficient allocation.

use crate::addr::PhysAddr;
use crate::mm::{FrameAllocator, FrameDeallocator, PhysMemoryRegion, PmmError};
use crate::paging::{PhysFrame, Size4KiB};
use crate::sync::SpinLock;

const FRAME_SIZE: u64 = 4096;
const BITS_PER_WORD: usize = 64;

struct BitmapAllocatorInner {
    /// Bitmap stored as a static mutable slice of u64 words in HHDM-mapped memory.
    bitmap: &'static mut [u64],
    /// Total number of frames tracked by the bitmap.
    total_frames: usize,
    /// Number of currently free frames.
    free_count: usize,
    /// Word index hint for next allocation search (amortized O(1)).
    search_hint: usize,
    /// HHDM offset for physical-to-virtual translation (used by page poisoning).
    hhdm_offset: u64,
}

// ---------------------------------------------------------------------------
// PMM page poisoning helpers (always defined for cfg!() type-checking)
// ---------------------------------------------------------------------------

/// Poison pattern written to freed pages: `0xDEAD_DEAD` repeated.
const PAGE_POISON_PATTERN: u32 = 0xDEAD_DEAD;

/// Writes the poison pattern across a 4 KiB page via HHDM.
fn poison_page(phys_addr: u64, hhdm_offset: u64) {
    let virt = (hhdm_offset + phys_addr) as *mut u32;
    for i in 0..(FRAME_SIZE as usize / 4) {
        // SAFETY: The virtual address is within the HHDM region and the page
        // has just been freed (no longer in use).
        unsafe { virt.add(i).write_volatile(PAGE_POISON_PATTERN) };
    }
}

/// Checks whether a previously poisoned page is still intact.
///
/// Returns `true` if the page was never poisoned (first word doesn't match)
/// or if the full poison pattern is intact. Returns `false` only when partial
/// corruption is detected (first word matches but later words don't),
/// indicating a use-after-free.
fn check_page_poison(phys_addr: u64, hhdm_offset: u64) -> bool {
    let virt = (hhdm_offset + phys_addr) as *const u32;
    // Quick check: if the first word isn't poison, page was never poisoned
    // (first allocation after boot). Skip verification.
    // SAFETY: The virtual address is within the HHDM region.
    if unsafe { virt.read_volatile() } != PAGE_POISON_PATTERN {
        return true;
    }
    // First word matches; verify the rest of the page.
    for i in 1..(FRAME_SIZE as usize / 4) {
        // SAFETY: The virtual address is within the HHDM region.
        if unsafe { virt.add(i).read_volatile() } != PAGE_POISON_PATTERN {
            return false;
        }
    }
    true
}

/// A bitmap-based physical frame allocator.
///
/// Uses interior mutability via [`SpinLock`] so all public methods take `&self`.
pub struct BitmapAllocator {
    inner: SpinLock<BitmapAllocatorInner>,
}

// SAFETY: The mutable slice in BitmapAllocatorInner is only accessed under the
// SpinLock, and u64 is Send. The SpinLock ensures mutual exclusion.
unsafe impl Send for BitmapAllocator {}
unsafe impl Sync for BitmapAllocator {}

impl BitmapAllocator {
    /// Creates a new bitmap allocator from a slice of physical memory regions.
    ///
    /// # Safety
    ///
    /// - `hhdm_offset` must be the correct HHDM offset.
    /// - `regions` must accurately describe physical memory.
    /// - This must be called exactly once during boot.
    pub unsafe fn new(regions: &[PhysMemoryRegion], hhdm_offset: u64) -> Result<Self, PmmError> {
        // 1. Find highest usable physical address to determine bitmap size.
        // We only need to track frames up to the end of the last usable region,
        // since we never allocate from non-usable regions.
        let max_phys = regions
            .iter()
            .filter(|r| r.usable)
            .map(|r| r.start.as_u64() + r.size)
            .max()
            .unwrap_or(0);

        if max_phys == 0 {
            return Err(PmmError::OutOfMemory);
        }

        let total_frames = (max_phys / FRAME_SIZE) as usize;
        let bitmap_words = (total_frames + BITS_PER_WORD - 1) / BITS_PER_WORD;
        let bitmap_bytes = bitmap_words * 8; // u64 = 8 bytes
        let bitmap_frame_count = (bitmap_bytes as u64 + FRAME_SIZE - 1) / FRAME_SIZE;

        // 2. Find the first usable region large enough for the bitmap.
        let bitmap_phys_start = regions
            .iter()
            .filter(|r| r.usable && r.size >= bitmap_bytes as u64)
            .map(|r| r.start)
            .next()
            .ok_or(PmmError::NoBitmapRegion)?;

        // 3. Map bitmap via HHDM and create a mutable slice.
        // SAFETY: The HHDM offset is valid, and bitmap_phys_start points to a
        // usable physical region large enough for bitmap_words * 8 bytes. The
        // region is not aliased because we are the sole consumer during boot.
        let bitmap = unsafe {
            let ptr = (hhdm_offset + bitmap_phys_start.as_u64()) as *mut u64;
            core::slice::from_raw_parts_mut(ptr, bitmap_words)
        };

        // 4. Set ALL bits to 1 (all frames reserved by default).
        bitmap.fill(u64::MAX);

        // 5. Clear bits for usable regions (mark them free).
        let mut free_count = 0usize;
        for region in regions.iter().filter(|r| r.usable) {
            let region_start_frame = (region.start.as_u64() / FRAME_SIZE) as usize;
            let region_frame_count = (region.size / FRAME_SIZE) as usize;

            for i in 0..region_frame_count {
                let frame_idx = region_start_frame + i;
                if frame_idx < total_frames {
                    let word_idx = frame_idx / BITS_PER_WORD;
                    let bit_idx = frame_idx % BITS_PER_WORD;
                    bitmap[word_idx] &= !(1u64 << bit_idx);
                    free_count += 1;
                }
            }
        }

        // 6. Re-set bits for the bitmap's own frames (they're now in use).
        let bitmap_start_frame = (bitmap_phys_start.as_u64() / FRAME_SIZE) as usize;
        for i in 0..bitmap_frame_count as usize {
            let frame_idx = bitmap_start_frame + i;
            if frame_idx < total_frames {
                let word_idx = frame_idx / BITS_PER_WORD;
                let bit_idx = frame_idx % BITS_PER_WORD;
                if bitmap[word_idx] & (1u64 << bit_idx) == 0 {
                    // Was marked free, now mark used
                    bitmap[word_idx] |= 1u64 << bit_idx;
                    free_count -= 1;
                }
            }
        }

        Ok(Self {
            inner: SpinLock::new(BitmapAllocatorInner {
                bitmap,
                total_frames,
                free_count,
                search_hint: 0,
                hhdm_offset,
            }),
        })
    }

    /// Allocates a single 4 KiB physical frame.
    pub fn allocate_frame(&self) -> Option<PhysFrame<Size4KiB>> {
        let mut inner = self.inner.lock();
        if inner.free_count == 0 {
            return None;
        }

        // Scan from search_hint, wrapping around if needed.
        let start = inner.search_hint;
        let words = inner.bitmap.len();

        for offset in 0..words {
            let word_idx = (start + offset) % words;
            let word = inner.bitmap[word_idx];

            // If all bits set, this word has no free frames.
            if word == u64::MAX {
                continue;
            }

            // Find first zero bit: invert, then trailing_zeros gives position.
            let bit_idx = (!word).trailing_zeros() as usize;
            let frame_idx = word_idx * BITS_PER_WORD + bit_idx;

            if frame_idx >= inner.total_frames {
                continue;
            }

            // Mark as allocated.
            inner.bitmap[word_idx] |= 1u64 << bit_idx;
            inner.free_count -= 1;
            inner.search_hint = word_idx;

            let phys_addr = frame_idx as u64 * FRAME_SIZE;

            // Verify poison pattern is intact (detects use-after-free).
            if cfg!(hadron_debug_pmm_poison)
                && !check_page_poison(phys_addr, inner.hhdm_offset)
            {
                crate::kwarn!(
                    "PMM: page at {:#x} modified after free (use-after-free)",
                    phys_addr
                );
            }

            crate::ktrace_subsys!(mm, "PMM: allocated frame at {:#x}", phys_addr);
            return Some(PhysFrame::containing_address(PhysAddr::new(phys_addr)));
        }

        None
    }

    /// Allocates `count` contiguous 4 KiB physical frames. Returns the first frame.
    pub fn allocate_frames(&self, count: usize) -> Option<PhysFrame<Size4KiB>> {
        if count == 0 {
            return None;
        }
        if count == 1 {
            return self.allocate_frame();
        }

        let mut inner = self.inner.lock();
        if inner.free_count < count {
            return None;
        }

        // Linear scan tracking consecutive free frames.
        let mut run_start = 0usize;
        let mut run_len = 0usize;

        let mut frame_idx = 0usize;
        while frame_idx < inner.total_frames {
            let word_idx = frame_idx / BITS_PER_WORD;
            let word = inner.bitmap[word_idx];

            if word == u64::MAX {
                // Entire word allocated, skip it.
                run_len = 0;
                frame_idx = (word_idx + 1) * BITS_PER_WORD;
                run_start = frame_idx;
                continue;
            }

            if word == 0 {
                // Entire word free, extend run by up to 64 frames.
                let extend =
                    core::cmp::min(BITS_PER_WORD, inner.total_frames - word_idx * BITS_PER_WORD);
                if run_len == 0 {
                    run_start = word_idx * BITS_PER_WORD;
                }
                run_len += extend;
                if run_len >= count {
                    break;
                }
                frame_idx = (word_idx + 1) * BITS_PER_WORD;
                continue;
            }

            // Partially occupied word -- check bit by bit.
            let bit_start = frame_idx % BITS_PER_WORD;
            for bit in bit_start..BITS_PER_WORD {
                let fi = word_idx * BITS_PER_WORD + bit;
                if fi >= inner.total_frames {
                    break;
                }
                if word & (1u64 << bit) != 0 {
                    // Allocated -- reset run.
                    run_len = 0;
                    run_start = fi + 1;
                } else {
                    if run_len == 0 {
                        run_start = fi;
                    }
                    run_len += 1;
                    if run_len >= count {
                        break;
                    }
                }
            }

            if run_len >= count {
                break;
            }
            frame_idx = (word_idx + 1) * BITS_PER_WORD;
        }

        if run_len < count {
            return None;
        }

        // Mark all frames in the run as allocated.
        for i in 0..count {
            let fi = run_start + i;
            let word_idx = fi / BITS_PER_WORD;
            let bit_idx = fi % BITS_PER_WORD;
            inner.bitmap[word_idx] |= 1u64 << bit_idx;

            // Verify poison pattern is intact (detects use-after-free).
            if cfg!(hadron_debug_pmm_poison) {
                let phys_addr = (fi as u64) * FRAME_SIZE;
                if !check_page_poison(phys_addr, inner.hhdm_offset) {
                    crate::kwarn!(
                        "PMM: page at {:#x} modified after free (use-after-free)",
                        phys_addr
                    );
                }
            }
        }
        inner.free_count -= count;
        inner.search_hint = (run_start + count) / BITS_PER_WORD;

        let phys = PhysAddr::new(run_start as u64 * FRAME_SIZE);
        Some(PhysFrame::containing_address(phys))
    }

    /// Deallocates a single 4 KiB physical frame.
    ///
    /// # Safety
    ///
    /// The frame must have been previously allocated by this allocator and
    /// must not be in use.
    pub unsafe fn deallocate_frame(&self, frame: PhysFrame<Size4KiB>) -> Result<(), PmmError> {
        let mut inner = self.inner.lock();
        let frame_idx = (frame.start_address().as_u64() / FRAME_SIZE) as usize;

        if frame_idx >= inner.total_frames {
            return Err(PmmError::InvalidFrame);
        }

        let word_idx = frame_idx / BITS_PER_WORD;
        let bit_idx = frame_idx % BITS_PER_WORD;

        debug_assert!(
            inner.bitmap[word_idx] & (1u64 << bit_idx) != 0,
            "double free of frame {:#x}",
            frame.start_address().as_u64()
        );
        inner.bitmap[word_idx] &= !(1u64 << bit_idx);
        inner.free_count += 1;

        crate::ktrace_subsys!(mm, "PMM: freed frame at {:#x}", frame.start_address().as_u64());

        // Poison the freed page so use-after-free is detectable on re-allocation.
        if cfg!(hadron_debug_pmm_poison) {
            poison_page(frame.start_address().as_u64(), inner.hhdm_offset);
        }

        // Update hint to potentially speed up next allocation.
        if word_idx < inner.search_hint {
            inner.search_hint = word_idx;
        }

        Ok(())
    }

    /// Deallocates `count` contiguous 4 KiB physical frames starting at `frame`.
    ///
    /// # Safety
    ///
    /// All frames in the range must have been previously allocated by this
    /// allocator and must not be in use.
    pub unsafe fn deallocate_frames(
        &self,
        frame: PhysFrame<Size4KiB>,
        count: usize,
    ) -> Result<(), PmmError> {
        let mut inner = self.inner.lock();
        let start_idx = (frame.start_address().as_u64() / FRAME_SIZE) as usize;

        if start_idx + count > inner.total_frames {
            return Err(PmmError::InvalidFrame);
        }

        for i in 0..count {
            let fi = start_idx + i;
            let word_idx = fi / BITS_PER_WORD;
            let bit_idx = fi % BITS_PER_WORD;
            debug_assert!(
                inner.bitmap[word_idx] & (1u64 << bit_idx) != 0,
                "double free of frame {:#x}",
                (fi as u64) * FRAME_SIZE
            );
            inner.bitmap[word_idx] &= !(1u64 << bit_idx);
            inner.free_count += 1;

            // Poison each freed page.
            if cfg!(hadron_debug_pmm_poison) {
                poison_page((fi as u64) * FRAME_SIZE, inner.hhdm_offset);
            }
        }

        let hint_word = start_idx / BITS_PER_WORD;
        if hint_word < inner.search_hint {
            inner.search_hint = hint_word;
        }

        Ok(())
    }

    /// Returns the number of free frames.
    pub fn free_frames(&self) -> usize {
        self.inner.lock().free_count
    }

    /// Returns the total number of tracked frames.
    pub fn total_frames(&self) -> usize {
        self.inner.lock().total_frames
    }
}

/// Wrapper that implements `FrameAllocator` for `&BitmapAllocator`.
///
/// This allows the bitmap allocator (which uses interior mutability) to be
/// used with APIs that require `&mut impl FrameAllocator<Size4KiB>`.
pub struct BitmapFrameAllocRef<'a>(pub &'a BitmapAllocator);

unsafe impl FrameAllocator<Size4KiB> for BitmapFrameAllocRef<'_> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.0.allocate_frame()
    }
}

unsafe impl FrameDeallocator<Size4KiB> for BitmapFrameAllocRef<'_> {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        let _ = unsafe { self.0.deallocate_frame(frame) };
    }
}

// ---------------------------------------------------------------------------
// Kernel-level PMM glue
// ---------------------------------------------------------------------------

use crate::boot::{BootInfo, MemoryRegionKind};

/// Global physical memory manager.
static PMM: SpinLock<Option<BitmapAllocator>> = SpinLock::named("PMM", None); // Lock level 1

/// Initializes the PMM from boot info.
///
/// Converts the bootloader memory map into `PhysMemoryRegion` descriptors
/// and creates the bitmap allocator.
pub fn init(boot_info: &impl BootInfo) {
    let hhdm_offset = boot_info.hhdm_offset();
    let memory_map = boot_info.memory_map();

    // Convert boot info regions to PhysMemoryRegion.
    // Use a stack buffer since we can't heap-allocate yet.
    let mut regions = [PhysMemoryRegion {
        start: PhysAddr::zero(),
        size: 0,
        usable: false,
    }; 256];
    let mut count = 0;

    for region in memory_map {
        if count >= regions.len() {
            break;
        }
        regions[count] = PhysMemoryRegion {
            start: region.start,
            size: region.size,
            usable: region.kind == MemoryRegionKind::Usable,
        };
        count += 1;
    }

    let allocator = unsafe {
        BitmapAllocator::new(&regions[..count], hhdm_offset).expect("failed to initialize PMM")
    };

    let mut pmm = PMM.lock();
    assert!(pmm.is_none(), "PMM already initialized");
    *pmm = Some(allocator);
}

/// Executes a closure with a reference to the global PMM.
///
/// # Panics
///
/// Panics if the PMM has not been initialized.
pub fn with_pmm<R>(f: impl FnOnce(&BitmapAllocator) -> R) -> R {
    let pmm = PMM.lock();
    f(pmm.as_ref().expect("PMM not initialized"))
}

/// Attempts to execute a closure with a reference to the global PMM.
///
/// Returns `None` if the PMM lock is already held (avoiding deadlock in
/// fault handlers) or if the PMM has not been initialized yet.
pub fn try_with_pmm<R>(f: impl FnOnce(&BitmapAllocator) -> R) -> Option<R> {
    let pmm = PMM.try_lock()?;
    Some(f(pmm.as_ref()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    const PAGE_SIZE: usize = FRAME_SIZE as usize;

    fn alloc_page() -> *mut u8 {
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        // SAFETY: layout is valid, non-zero size.
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        ptr
    }

    unsafe fn free_page(ptr: *mut u8) {
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        unsafe { std::alloc::dealloc(ptr, layout) };
    }

    #[test]
    fn test_poison_page_writes_pattern() {
        let buf = alloc_page();
        // poison_page expects a physical address and hhdm_offset such that
        // hhdm_offset + phys_addr = virtual address. We set phys=0, hhdm=buf.
        poison_page(0, buf as u64);

        let words = unsafe { core::slice::from_raw_parts(buf as *const u32, PAGE_SIZE / 4) };
        assert!(words.iter().all(|&w| w == PAGE_POISON_PATTERN));

        unsafe { free_page(buf) };
    }

    #[test]
    fn test_check_page_poison_intact() {
        let buf = alloc_page();
        poison_page(0, buf as u64);
        assert!(check_page_poison(0, buf as u64));
        unsafe { free_page(buf) };
    }

    #[test]
    fn test_check_page_poison_never_poisoned() {
        let buf = alloc_page();
        // Zero-filled page: first word is 0, not the poison pattern.
        // Heuristic returns true (assumes never-poisoned).
        assert!(check_page_poison(0, buf as u64));
        unsafe { free_page(buf) };
    }

    #[test]
    fn test_check_page_poison_partial_corruption() {
        let buf = alloc_page();
        poison_page(0, buf as u64);

        // Corrupt a word in the middle of the page.
        let words = buf as *mut u32;
        unsafe { words.add(512).write_volatile(0x0) };

        assert!(!check_page_poison(0, buf as u64));
        unsafe { free_page(buf) };
    }

    #[test]
    fn test_check_page_poison_first_word_zero() {
        let buf = alloc_page();
        poison_page(0, buf as u64);

        // Overwrite first word with 0 — heuristic thinks page was never
        // poisoned, so it returns true (skip verification).
        let words = buf as *mut u32;
        unsafe { words.write_volatile(0x0) };

        assert!(check_page_poison(0, buf as u64));
        unsafe { free_page(buf) };
    }
}
