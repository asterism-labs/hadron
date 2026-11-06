//! A frame allocator that hands out one physical range in order.
//!
//! Boot uses it before the bitmap exists. It never frees, so everything it
//! hands out stays in use. The bitmap marks [`BumpAllocator::used`] as
//! allocated when it takes over.

use arch_x86_64::PhysAddr;
use arch_x86_64::structures::paging::PAGE_SIZE;

use crate::frame::{FrameAllocator, PhysFrame};

/// Hands out frames from `[next, end)`, lowest first.
#[derive(Debug)]
pub struct BumpAllocator {
    start: PhysAddr,
    next: PhysAddr,
    end: PhysAddr,
}

impl BumpAllocator {
    /// An allocator over the whole frames in `[start, end)`.
    #[must_use]
    pub const fn new(start: PhysAddr, end: PhysAddr) -> Self {
        let start = start.align_up(PAGE_SIZE);
        let end = end.align_down(PAGE_SIZE);
        let end = if end.as_u64() < start.as_u64() {
            start
        } else {
            end
        };
        Self {
            start,
            next: start,
            end,
        }
    }

    /// `count` contiguous frames, or `None` if fewer are left. Returns the
    /// first frame.
    pub fn allocate_frames(&mut self, count: u64) -> Option<PhysFrame> {
        let bytes = count.checked_mul(PAGE_SIZE)?;
        if self.end - self.next < bytes {
            return None;
        }
        // `next` stays 4 KiB aligned.
        let first = PhysFrame::containing(self.next);
        self.next = self.next + bytes;
        Some(first)
    }

    /// The range handed out so far, `[start, next)`.
    #[must_use]
    pub const fn used(&self) -> (PhysAddr, PhysAddr) {
        (self.start, self.next)
    }

    /// Frames not handed out yet.
    #[must_use]
    pub fn remaining_frames(&self) -> u64 {
        (self.end - self.next) / PAGE_SIZE
    }
}

impl FrameAllocator for BumpAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.allocate_frames(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_out_frames_in_order_until_the_end() {
        let mut bump = BumpAllocator::new(PhysAddr::new(0x1000), PhysAddr::new(0x3000));
        assert_eq!(bump.allocate_frame(), Some(PhysFrame::from_number(1)));
        assert_eq!(bump.allocate_frame(), Some(PhysFrame::from_number(2)));
        assert_eq!(bump.allocate_frame(), None);
        assert_eq!(bump.used(), (PhysAddr::new(0x1000), PhysAddr::new(0x3000)));
    }

    #[test]
    fn rounds_the_range_inward() {
        let mut bump = BumpAllocator::new(PhysAddr::new(0x1001), PhysAddr::new(0x3fff));
        assert_eq!(bump.remaining_frames(), 1);
        assert_eq!(bump.allocate_frame(), Some(PhysFrame::from_number(2)));
    }

    #[test]
    fn a_range_smaller_than_a_frame_is_empty() {
        let mut bump = BumpAllocator::new(PhysAddr::new(0x1800), PhysAddr::new(0x1900));
        assert_eq!(bump.remaining_frames(), 0);
        assert_eq!(bump.allocate_frame(), None);
    }

    #[test]
    fn contiguous_request_fails_without_consuming() {
        let mut bump = BumpAllocator::new(PhysAddr::new(0), PhysAddr::new(0x3000));
        assert_eq!(bump.allocate_frames(4), None);
        assert_eq!(bump.allocate_frames(3), Some(PhysFrame::from_number(0)));
    }
}
