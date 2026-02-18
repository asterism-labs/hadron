//! Bitmap-based physical frame allocator.
//!
//! Uses a bitmap stored in HHDM-accessible memory where each bit represents
//! one 4 KiB frame. Bit = 1 means allocated/reserved, bit = 0 means free.
//! Word-level scanning with `trailing_zeros()` (compiles to TZCNT/BSF on
//! x86_64) provides efficient allocation.

use core::ptr;

use crate::addr::PhysAddr;
use crate::mm::{FrameAllocator, FrameDeallocator, PhysMemoryRegion, PmmError};
use crate::paging::{PhysFrame, Size4KiB};
use crate::sync::SpinLock;

const FRAME_SIZE: u64 = 4096;
const BITS_PER_WORD: usize = 64;

struct BitmapAllocatorInner {
    /// Pointer to the bitmap in HHDM-mapped memory (array of u64 words).
    bitmap: *mut u64,
    /// Total number of frames tracked by the bitmap.
    total_frames: usize,
    /// Number of u64 words in the bitmap.
    bitmap_words: usize,
    /// Number of currently free frames.
    free_count: usize,
    /// Word index hint for next allocation search (amortized O(1)).
    search_hint: usize,
}

// SAFETY: The bitmap pointer is accessed only under the SpinLock.
unsafe impl Send for BitmapAllocatorInner {}

/// A bitmap-based physical frame allocator.
///
/// Uses interior mutability via [`SpinLock`] so all public methods take `&self`.
pub struct BitmapAllocator {
    inner: SpinLock<BitmapAllocatorInner>,
}

// SAFETY: Protected by SpinLock.
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

        // 3. Map bitmap via HHDM and zero it.
        let bitmap_virt = (hhdm_offset + bitmap_phys_start.as_u64()) as *mut u64;

        // 4. Set ALL bits to 1 (all frames reserved by default).
        unsafe {
            ptr::write_bytes(bitmap_virt, 0xFF, bitmap_words);
        }

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
                    unsafe {
                        let word = bitmap_virt.add(word_idx);
                        *word &= !(1u64 << bit_idx);
                    }
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
                unsafe {
                    let word = bitmap_virt.add(word_idx);
                    if *word & (1u64 << bit_idx) == 0 {
                        // Was marked free, now mark used
                        *word |= 1u64 << bit_idx;
                        free_count -= 1;
                    }
                }
            }
        }

        Ok(Self {
            inner: SpinLock::new(BitmapAllocatorInner {
                bitmap: bitmap_virt,
                total_frames,
                bitmap_words,
                free_count,
                search_hint: 0,
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
        let words = inner.bitmap_words;

        for offset in 0..words {
            let word_idx = (start + offset) % words;
            let word = unsafe { *inner.bitmap.add(word_idx) };

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
            unsafe {
                *inner.bitmap.add(word_idx) |= 1u64 << bit_idx;
            }
            inner.free_count -= 1;
            inner.search_hint = word_idx;

            let phys = PhysAddr::new(frame_idx as u64 * FRAME_SIZE);
            return Some(PhysFrame::containing_address(phys));
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
            let word = unsafe { *inner.bitmap.add(word_idx) };

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
            unsafe {
                *inner.bitmap.add(word_idx) |= 1u64 << bit_idx;
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

        unsafe {
            let word = inner.bitmap.add(word_idx);
            debug_assert!(
                *word & (1u64 << bit_idx) != 0,
                "double free of frame {:#x}",
                frame.start_address().as_u64()
            );
            *word &= !(1u64 << bit_idx);
        }
        inner.free_count += 1;

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
            unsafe {
                let word = inner.bitmap.add(word_idx);
                debug_assert!(
                    *word & (1u64 << bit_idx) != 0,
                    "double free of frame {:#x}",
                    (fi as u64) * FRAME_SIZE
                );
                *word &= !(1u64 << bit_idx);
            }
            inner.free_count += 1;
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
