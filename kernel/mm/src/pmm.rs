//! Bitmap-based physical frame allocator.
//!
//! Uses a bitmap stored in HHDM-accessible memory where each bit represents
//! one 4 KiB frame. Bit = 1 means allocated/reserved, bit = 0 means free.
//! Word-level scanning with `trailing_zeros()` (compiles to TZCNT/BSF on
//! x86_64) provides efficient allocation.

use hadron_core::addr::PhysAddr;
use hadron_core::paging::{PhysFrame, Size4KiB};
use hadron_core::sync::SpinLock;

use crate::{FrameAllocator, FrameDeallocator, PhysMemoryRegion, PmmError};

const FRAME_SIZE: u64 = 4096;
const BITS_PER_WORD: usize = 64;

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
/// All mutation goes through `&mut self`; the outer `PMM: SpinLock<Option<…>>`
/// provides thread safety, so no interior `SpinLock` is needed.
pub struct BitmapAllocator {
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
            bitmap,
            total_frames,
            free_count,
            search_hint: 0,
            hhdm_offset,
        })
    }

    /// Allocates a single 4 KiB physical frame.
    pub fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        if self.free_count == 0 {
            return None;
        }

        // Scan from search_hint, wrapping around if needed.
        let start = self.search_hint;
        let words = self.bitmap.len();

        for offset in 0..words {
            let word_idx = (start + offset) % words;
            let word = self.bitmap[word_idx];

            // If all bits set, this word has no free frames.
            if word == u64::MAX {
                continue;
            }

            // Find first zero bit: invert, then trailing_zeros gives position.
            let bit_idx = (!word).trailing_zeros() as usize;
            let frame_idx = word_idx * BITS_PER_WORD + bit_idx;

            if frame_idx >= self.total_frames {
                continue;
            }

            // Mark as allocated.
            self.bitmap[word_idx] |= 1u64 << bit_idx;
            self.free_count -= 1;
            self.search_hint = word_idx;

            let phys_addr = frame_idx as u64 * FRAME_SIZE;

            // Verify poison pattern is intact (detects use-after-free).
            // NOTE: No logging here — PMM lock is held and logging would
            // acquire LOGGER, creating a PMM → LOGGER lock ordering violation.
            if cfg!(hadron_debug_pmm_poison) && !check_page_poison(phys_addr, self.hhdm_offset) {
                panic!(
                    "PMM: page at {:#x} modified after free (use-after-free)",
                    phys_addr
                );
            }

            return Some(PhysFrame::containing_address(PhysAddr::new(phys_addr)));
        }

        None
    }

    /// Allocates `count` contiguous 4 KiB physical frames. Returns the first frame.
    pub fn allocate_frames(&mut self, count: usize) -> Option<PhysFrame<Size4KiB>> {
        if count == 0 {
            return None;
        }
        if count == 1 {
            return self.allocate_frame();
        }

        if self.free_count < count {
            return None;
        }

        // Linear scan tracking consecutive free frames.
        let mut run_start = 0usize;
        let mut run_len = 0usize;

        let mut frame_idx = 0usize;
        while frame_idx < self.total_frames {
            let word_idx = frame_idx / BITS_PER_WORD;
            let word = self.bitmap[word_idx];

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
                    core::cmp::min(BITS_PER_WORD, self.total_frames - word_idx * BITS_PER_WORD);
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
                if fi >= self.total_frames {
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
            self.bitmap[word_idx] |= 1u64 << bit_idx;

            // Verify poison pattern is intact (detects use-after-free).
            // NOTE: No logging here — PMM lock is held (see allocate_frame).
            if cfg!(hadron_debug_pmm_poison) {
                let phys_addr = (fi as u64) * FRAME_SIZE;
                if !check_page_poison(phys_addr, self.hhdm_offset) {
                    panic!(
                        "PMM: page at {:#x} modified after free (use-after-free)",
                        phys_addr
                    );
                }
            }
        }
        self.free_count -= count;
        self.search_hint = (run_start + count) / BITS_PER_WORD;

        let phys = PhysAddr::new(run_start as u64 * FRAME_SIZE);
        Some(PhysFrame::containing_address(phys))
    }

    /// Deallocates a single 4 KiB physical frame.
    ///
    /// # Safety
    ///
    /// The frame must have been previously allocated by this allocator and
    /// must not be in use.
    pub unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) -> Result<(), PmmError> {
        let frame_idx = (frame.start_address().as_u64() / FRAME_SIZE) as usize;

        if frame_idx >= self.total_frames {
            return Err(PmmError::InvalidFrame);
        }

        let word_idx = frame_idx / BITS_PER_WORD;
        let bit_idx = frame_idx % BITS_PER_WORD;

        debug_assert!(
            self.bitmap[word_idx] & (1u64 << bit_idx) != 0,
            "double free of frame {:#x}",
            frame.start_address().as_u64()
        );
        self.bitmap[word_idx] &= !(1u64 << bit_idx);
        self.free_count += 1;

        // Poison the freed page so use-after-free is detectable on re-allocation.
        if cfg!(hadron_debug_pmm_poison) {
            poison_page(frame.start_address().as_u64(), self.hhdm_offset);
        }

        // Update hint to potentially speed up next allocation.
        if word_idx < self.search_hint {
            self.search_hint = word_idx;
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
        &mut self,
        frame: PhysFrame<Size4KiB>,
        count: usize,
    ) -> Result<(), PmmError> {
        let start_idx = (frame.start_address().as_u64() / FRAME_SIZE) as usize;

        if start_idx + count > self.total_frames {
            return Err(PmmError::InvalidFrame);
        }

        for i in 0..count {
            let fi = start_idx + i;
            let word_idx = fi / BITS_PER_WORD;
            let bit_idx = fi % BITS_PER_WORD;
            debug_assert!(
                self.bitmap[word_idx] & (1u64 << bit_idx) != 0,
                "double free of frame {:#x}",
                (fi as u64) * FRAME_SIZE
            );
            self.bitmap[word_idx] &= !(1u64 << bit_idx);
            self.free_count += 1;

            // Poison each freed page.
            if cfg!(hadron_debug_pmm_poison) {
                poison_page((fi as u64) * FRAME_SIZE, self.hhdm_offset);
            }
        }

        let hint_word = start_idx / BITS_PER_WORD;
        if hint_word < self.search_hint {
            self.search_hint = hint_word;
        }

        Ok(())
    }

    /// Returns the number of free frames.
    pub fn free_frames(&self) -> usize {
        self.free_count
    }

    /// Returns the total number of tracked frames.
    pub fn total_frames(&self) -> usize {
        self.total_frames
    }
}

/// Wrapper that implements `FrameAllocator` / `FrameDeallocator` by
/// forwarding to `&mut BitmapAllocator`.
pub struct BitmapFrameAllocRef<'a>(pub &'a mut BitmapAllocator);

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
// Global PMM
// ---------------------------------------------------------------------------

/// Global physical memory manager.
static PMM: SpinLock<Option<BitmapAllocator>> = SpinLock::leveled("PMM", 3, None);

/// Initializes the PMM from a slice of physical memory regions.
///
/// The caller is responsible for converting bootloader-specific memory maps
/// into [`PhysMemoryRegion`] descriptors before calling this function.
pub fn init(regions: &[PhysMemoryRegion], hhdm_offset: u64) {
    let allocator =
        unsafe { BitmapAllocator::new(regions, hhdm_offset).expect("failed to initialize PMM") };

    let mut pmm = PMM.lock();
    assert!(pmm.is_none(), "PMM already initialized");
    *pmm = Some(allocator);
}

/// Executes a closure with an exclusive reference to the global PMM.
///
/// # Panics
///
/// Panics if the PMM has not been initialized.
pub fn with<R>(f: impl FnOnce(&mut BitmapAllocator) -> R) -> R {
    let mut pmm = PMM.lock();
    f(pmm.as_mut().expect("PMM not initialized"))
}

/// Attempts to execute a closure with an exclusive reference to the global PMM.
///
/// Returns `None` if the PMM lock is already held (avoiding deadlock in
/// fault handlers) or if the PMM has not been initialized yet.
pub fn try_with<R>(f: impl FnOnce(&mut BitmapAllocator) -> R) -> Option<R> {
    let mut pmm = PMM.try_lock()?;
    Some(f(pmm.as_mut()?))
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
