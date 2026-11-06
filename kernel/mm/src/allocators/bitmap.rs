//! The physical frame allocator: one bit per 4 KiB frame.
//!
//! A set bit means the frame is in use. The allocator starts with every
//! frame in use. Boot then marks the usable regions free and marks the
//! frames the bump allocator handed out as used again.
//!
//! Single frames come from a scan that starts at the lowest word that may
//! have a clear bit, so a run of full words is skipped once, not on every
//! call. Contiguous runs come from a linear scan.

use arch_x86_64::PhysAddr;
use arch_x86_64::structures::paging::PAGE_SIZE;

use crate::frame::{FrameAllocator, PhysFrame};

/// Frames tracked by one word.
const WORD_BITS: u64 = u64::BITS as u64;

/// Why [`BitmapFrameAllocator::free`] refused a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeError {
    /// The frame is past the last frame the bitmap tracks.
    OutOfRange,
    /// The frame was already free.
    NotAllocated,
}

/// A bitmap over frames `0..frame_count`.
#[derive(Debug)]
pub struct BitmapFrameAllocator<'a> {
    words: &'a mut [u64],
    frame_count: u64,
    free_frames: u64,
    /// No word below this index has a clear bit.
    hint: usize,
}

impl<'a> BitmapFrameAllocator<'a> {
    /// Words of storage needed to track `frame_count` frames.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // fits on a 64-bit target
    pub const fn words_for(frame_count: u64) -> usize {
        frame_count.div_ceil(WORD_BITS) as usize
    }

    /// A bitmap over `frame_count` frames with every frame in use.
    ///
    /// # Panics
    ///
    /// Panics if `words` holds fewer than [`words_for`](Self::words_for)
    /// words.
    pub fn new(words: &'a mut [u64], frame_count: u64) -> Self {
        let needed = Self::words_for(frame_count);
        assert!(words.len() >= needed, "bitmap storage is too small");
        let words = &mut words[..needed];
        words.fill(u64::MAX);
        Self {
            words,
            frame_count,
            free_frames: 0,
            hint: needed,
        }
    }

    /// Frames the bitmap tracks.
    #[must_use]
    pub const fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Frames currently free.
    #[must_use]
    pub const fn free_frames(&self) -> u64 {
        self.free_frames
    }

    /// Whether `frame` is in use. Frames past the end count as in use.
    #[must_use]
    pub fn is_used(&self, frame: PhysFrame) -> bool {
        let number = frame.number();
        number >= self.frame_count || self.bit(number)
    }

    /// Marks the whole frames inside `[start, end)` free. Frames past the
    /// end of the bitmap are ignored.
    pub fn mark_free(&mut self, start: PhysAddr, end: PhysAddr) {
        let first = start.align_up(PAGE_SIZE).as_u64() / PAGE_SIZE;
        let last = (end.align_down(PAGE_SIZE).as_u64() / PAGE_SIZE).min(self.frame_count);
        for number in first..last {
            if self.bit(number) {
                self.set_bit(number, false);
                self.free_frames += 1;
            }
        }
        if first < last {
            self.hint = self.hint.min(Self::word_index(first));
        }
    }

    /// Marks every frame that overlaps `[start, end)` in use. Frames past
    /// the end of the bitmap are ignored.
    pub fn mark_used(&mut self, start: PhysAddr, end: PhysAddr) {
        let first = start.align_down(PAGE_SIZE).as_u64() / PAGE_SIZE;
        let last = (end.as_u64().div_ceil(PAGE_SIZE)).min(self.frame_count);
        for number in first..last {
            if !self.bit(number) {
                self.set_bit(number, true);
                self.free_frames -= 1;
            }
        }
    }

    /// A free frame, marked in use, or `None` if every frame is in use.
    pub fn allocate(&mut self) -> Option<PhysFrame> {
        let index = self.hint
            + self.words[self.hint..]
                .iter()
                .position(|&w| w != u64::MAX)?;
        self.hint = index;
        let bit = u64::from(self.words[index].trailing_ones());
        let number = index as u64 * WORD_BITS + bit;
        self.set_bit(number, true);
        self.free_frames -= 1;
        Some(PhysFrame::from_number(number))
    }

    /// `count` contiguous free frames whose first frame number is a
    /// multiple of `align`, marked in use. Returns the first frame.
    ///
    /// # Panics
    ///
    /// Panics if `count` is 0 or `align` is not a power of two.
    pub fn allocate_contiguous(&mut self, count: u64, align: u64) -> Option<PhysFrame> {
        assert!(count > 0, "allocate_contiguous of 0 frames");
        assert!(align.is_power_of_two(), "alignment is not a power of two");
        let mut start = (self.hint as u64 * WORD_BITS).next_multiple_of(align);
        while start + count <= self.frame_count {
            // Restart past the highest used frame in the window, since no
            // run that holds it fits.
            if let Some(used) = (start..start + count).rev().find(|&n| self.bit(n)) {
                start = (used + 1).next_multiple_of(align);
            } else {
                for number in start..start + count {
                    self.set_bit(number, true);
                }
                self.free_frames -= count;
                return Some(PhysFrame::from_number(start));
            }
        }
        None
    }

    /// Returns `frame` to the free pool.
    ///
    /// # Errors
    ///
    /// Fails if the frame is past the end or already free. Both mean the
    /// caller has a bug, so the bitmap stays unchanged.
    pub fn free(&mut self, frame: PhysFrame) -> Result<(), FreeError> {
        let number = frame.number();
        if number >= self.frame_count {
            return Err(FreeError::OutOfRange);
        }
        if !self.bit(number) {
            return Err(FreeError::NotAllocated);
        }
        self.set_bit(number, false);
        self.free_frames += 1;
        self.hint = self.hint.min(Self::word_index(number));
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation)] // fits on a 64-bit target
    const fn word_index(number: u64) -> usize {
        (number / WORD_BITS) as usize
    }

    fn bit(&self, number: u64) -> bool {
        self.words[Self::word_index(number)] & (1 << (number % WORD_BITS)) != 0
    }

    fn set_bit(&mut self, number: u64, used: bool) {
        let word = &mut self.words[Self::word_index(number)];
        let mask = 1 << (number % WORD_BITS);
        if used {
            *word |= mask;
        } else {
            *word &= !mask;
        }
    }
}

impl FrameAllocator for BitmapFrameAllocator<'_> {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.allocate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(first: u64, last: u64) -> (PhysAddr, PhysAddr) {
        (
            PhysAddr::new(first * PAGE_SIZE),
            PhysAddr::new(last * PAGE_SIZE),
        )
    }

    fn bitmap(storage: &mut [u64], count: u64) -> BitmapFrameAllocator<'_> {
        BitmapFrameAllocator::new(storage, count)
    }

    #[test]
    fn starts_with_every_frame_in_use() {
        let mut storage = [0; 2];
        let mut map = bitmap(&mut storage, 100);
        assert_eq!(map.free_frames(), 0);
        assert_eq!(map.allocate(), None);
    }

    #[test]
    fn allocates_the_lowest_free_frame() {
        let mut storage = [0; 2];
        let mut map = bitmap(&mut storage, 128);
        let (start, end) = frames(70, 73);
        map.mark_free(start, end);
        assert_eq!(map.free_frames(), 3);
        assert_eq!(map.allocate(), Some(PhysFrame::from_number(70)));
        assert_eq!(map.allocate(), Some(PhysFrame::from_number(71)));
        assert_eq!(map.allocate(), Some(PhysFrame::from_number(72)));
        assert_eq!(map.allocate(), None);
    }

    #[test]
    fn freed_frame_is_reused() {
        let mut storage = [0; 1];
        let mut map = bitmap(&mut storage, 64);
        let (start, end) = frames(0, 64);
        map.mark_free(start, end);
        let a = map.allocate().unwrap();
        let _b = map.allocate().unwrap();
        map.free(a).unwrap();
        assert_eq!(map.allocate(), Some(a));
    }

    #[test]
    fn free_rejects_double_free_and_out_of_range() {
        let mut storage = [0; 1];
        let mut map = bitmap(&mut storage, 10);
        let (start, end) = frames(0, 10);
        map.mark_free(start, end);
        assert_eq!(
            map.free(PhysFrame::from_number(3)),
            Err(FreeError::NotAllocated)
        );
        assert_eq!(
            map.free(PhysFrame::from_number(10)),
            Err(FreeError::OutOfRange)
        );
        assert_eq!(map.free_frames(), 10);
    }

    #[test]
    fn frames_past_the_count_are_never_handed_out() {
        let mut storage = [0; 1];
        let mut map = bitmap(&mut storage, 3);
        let (start, end) = frames(0, 64);
        map.mark_free(start, end);
        assert_eq!(map.free_frames(), 3);
        for _ in 0..3 {
            assert!(map.allocate().is_some());
        }
        assert_eq!(map.allocate(), None);
    }

    #[test]
    fn mark_free_rounds_inward_and_mark_used_rounds_outward() {
        let mut storage = [0; 1];
        let mut map = bitmap(&mut storage, 64);
        map.mark_free(PhysAddr::new(0x1800), PhysAddr::new(0x4800));
        assert_eq!(map.free_frames(), 2); // frames 2 and 3
        map.mark_used(PhysAddr::new(0x2fff), PhysAddr::new(0x3001));
        assert_eq!(map.free_frames(), 0);
    }

    #[test]
    fn contiguous_run_skips_holes_and_respects_alignment() {
        let mut storage = [0; 2];
        let mut map = bitmap(&mut storage, 128);
        let (start, end) = frames(1, 128);
        map.mark_free(start, end);
        let (start, end) = frames(9, 10);
        map.mark_used(start, end);
        // 8 frames aligned to 8: 0..8 holds used frame 0, 8..16 holds 9.
        assert_eq!(
            map.allocate_contiguous(8, 8),
            Some(PhysFrame::from_number(16))
        );
        assert!((16..24).all(|n| map.is_used(PhysFrame::from_number(n))));
        assert_eq!(map.free_frames(), 127 - 1 - 8);
        // A run of 3 fits at 1 with no alignment.
        assert_eq!(
            map.allocate_contiguous(3, 1),
            Some(PhysFrame::from_number(1))
        );
    }

    #[test]
    fn contiguous_run_that_does_not_fit_fails() {
        let mut storage = [0; 1];
        let mut map = bitmap(&mut storage, 16);
        let (start, end) = frames(0, 16);
        map.mark_free(start, end);
        assert_eq!(map.allocate_contiguous(17, 1), None);
        assert_eq!(map.free_frames(), 16);
    }

    #[test]
    #[should_panic(expected = "too small")]
    fn rejects_short_storage() {
        let mut storage = [0; 1];
        let _ = bitmap(&mut storage, 65);
    }
}
