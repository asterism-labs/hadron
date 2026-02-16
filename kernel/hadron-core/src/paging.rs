//! Page size traits and typed page/frame abstractions.
//!
//! Provides generic [`Page<S>`] and [`PhysFrame<S>`] types parameterised over
//! a [`PageSize`], preventing accidental misuse of differently-sized pages.

use core::fmt;
use core::iter::FusedIterator;
use core::ops::{Add, Sub};

use crate::addr::{PhysAddr, VirtAddr};

/// Trait for page sizes (4 KiB, 2 MiB, 1 GiB).
pub trait PageSize: Copy + Eq + PartialOrd + Ord {
    /// The size in bytes.
    const SIZE: u64;
    /// Human-readable size string for debug output.
    const SIZE_AS_DEBUG_STR: &'static str;
}

/// 4 KiB page size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Size4KiB;

impl PageSize for Size4KiB {
    const SIZE: u64 = 4096;
    const SIZE_AS_DEBUG_STR: &'static str = "4KiB";
}

/// 2 MiB page size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Size2MiB;

impl PageSize for Size2MiB {
    const SIZE: u64 = 0x20_0000;
    const SIZE_AS_DEBUG_STR: &'static str = "2MiB";
}

/// 1 GiB page size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Size1GiB;

impl PageSize for Size1GiB {
    const SIZE: u64 = 0x4000_0000;
    const SIZE_AS_DEBUG_STR: &'static str = "1GiB";
}

// ---------------------------------------------------------------------------
// Page<S>
// ---------------------------------------------------------------------------

/// A virtual memory page of size `S`.
///
/// The contained [`VirtAddr`] is guaranteed to be aligned to `S::SIZE`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Page<S: PageSize> {
    start: VirtAddr,
    _marker: core::marker::PhantomData<S>,
}

/// Error type returned when an address is not properly aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressNotAligned;

impl<S: PageSize> Page<S> {
    /// Returns the page that contains the given virtual address (aligns down).
    #[inline]
    pub fn containing_address(addr: VirtAddr) -> Self {
        Self {
            start: addr.align_down(S::SIZE),
            _marker: core::marker::PhantomData,
        }
    }

    /// Creates a page from an already-aligned start address.
    ///
    /// Returns `Err(AddressNotAligned)` if the address is not aligned to the
    /// page size.
    #[inline]
    pub fn from_start_address(addr: VirtAddr) -> Result<Self, AddressNotAligned> {
        if !addr.is_aligned(S::SIZE) {
            return Err(AddressNotAligned);
        }
        Ok(Self {
            start: addr,
            _marker: core::marker::PhantomData,
        })
    }

    /// Returns the start address of this page.
    #[inline]
    pub const fn start_address(&self) -> VirtAddr {
        self.start
    }

    /// Returns the page size in bytes.
    #[inline]
    pub const fn size(&self) -> u64 {
        S::SIZE
    }

    /// Creates an iterator over a range of pages `[start, end)`.
    #[inline]
    pub fn range(start: Page<S>, end: Page<S>) -> PageRange<S> {
        PageRange { start, end }
    }
}

impl<S: PageSize> Add<u64> for Page<S> {
    type Output = Self;
    #[inline]
    fn add(self, rhs: u64) -> Self {
        Page::containing_address(self.start + rhs * S::SIZE)
    }
}

impl<S: PageSize> Sub<u64> for Page<S> {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: u64) -> Self {
        Page::containing_address(self.start - rhs * S::SIZE)
    }
}

impl<S: PageSize> fmt::Debug for Page<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Page[{}]({:#x})",
            S::SIZE_AS_DEBUG_STR,
            self.start.as_u64()
        )
    }
}

// ---------------------------------------------------------------------------
// PhysFrame<S>
// ---------------------------------------------------------------------------

/// A physical memory frame of size `S`.
///
/// The contained [`PhysAddr`] is guaranteed to be aligned to `S::SIZE`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysFrame<S: PageSize> {
    start: PhysAddr,
    _marker: core::marker::PhantomData<S>,
}

impl<S: PageSize> PhysFrame<S> {
    /// Returns the frame that contains the given physical address (aligns
    /// down).
    #[inline]
    pub fn containing_address(addr: PhysAddr) -> Self {
        Self {
            start: addr.align_down(S::SIZE),
            _marker: core::marker::PhantomData,
        }
    }

    /// Creates a frame from an already-aligned start address.
    ///
    /// Returns `Err(AddressNotAligned)` if the address is not aligned to the
    /// frame size.
    #[inline]
    pub fn from_start_address(addr: PhysAddr) -> Result<Self, AddressNotAligned> {
        if !addr.is_aligned(S::SIZE) {
            return Err(AddressNotAligned);
        }
        Ok(Self {
            start: addr,
            _marker: core::marker::PhantomData,
        })
    }

    /// Returns the start address of this frame.
    #[inline]
    pub const fn start_address(&self) -> PhysAddr {
        self.start
    }

    /// Returns the frame size in bytes.
    #[inline]
    pub const fn size(&self) -> u64 {
        S::SIZE
    }

    /// Creates an iterator over a range of frames `[start, end)`.
    #[inline]
    pub fn range(start: PhysFrame<S>, end: PhysFrame<S>) -> PhysFrameRange<S> {
        PhysFrameRange { start, end }
    }
}

impl<S: PageSize> Add<u64> for PhysFrame<S> {
    type Output = Self;
    #[inline]
    fn add(self, rhs: u64) -> Self {
        PhysFrame::containing_address(self.start + rhs * S::SIZE)
    }
}

impl<S: PageSize> Sub<u64> for PhysFrame<S> {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: u64) -> Self {
        PhysFrame::containing_address(self.start - rhs * S::SIZE)
    }
}

impl<S: PageSize> fmt::Debug for PhysFrame<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PhysFrame[{}]({:#x})",
            S::SIZE_AS_DEBUG_STR,
            self.start.as_u64()
        )
    }
}

// ---------------------------------------------------------------------------
// Range iterators
// ---------------------------------------------------------------------------

/// An iterator over a range of [`Page`]s.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageRange<S: PageSize> {
    start: Page<S>,
    end: Page<S>,
}

impl<S: PageSize> Iterator for PageRange<S> {
    type Item = Page<S>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.start.start.as_u64() < self.end.start.as_u64() {
            let page = self.start;
            self.start = self.start + 1;
            Some(page)
        } else {
            None
        }
    }
}

impl<S: PageSize> FusedIterator for PageRange<S> {}

/// An iterator over a range of [`PhysFrame`]s.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PhysFrameRange<S: PageSize> {
    start: PhysFrame<S>,
    end: PhysFrame<S>,
}

impl<S: PageSize> Iterator for PhysFrameRange<S> {
    type Item = PhysFrame<S>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.start.start.as_u64() < self.end.start.as_u64() {
            let frame = self.start;
            self.start = self.start + 1;
            Some(frame)
        } else {
            None
        }
    }
}

impl<S: PageSize> FusedIterator for PhysFrameRange<S> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::{PhysAddr, VirtAddr};

    #[test]
    fn page_containing_address() {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(0x1234));
        assert_eq!(page.start_address().as_u64(), 0x1000);
        assert_eq!(page.size(), 4096);
    }

    #[test]
    fn page_from_start_aligned() {
        let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(0x2000));
        assert!(page.is_ok());
        assert_eq!(page.unwrap().start_address().as_u64(), 0x2000);
    }

    #[test]
    fn page_from_start_unaligned() {
        let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(0x2001));
        assert_eq!(page.unwrap_err(), AddressNotAligned);
    }

    #[test]
    fn page_add_sub() {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(0x1000));
        assert_eq!((page + 3).start_address().as_u64(), 0x4000);
        assert_eq!((page + 3 - 1).start_address().as_u64(), 0x3000);
    }

    #[test]
    fn page_2mib() {
        let page = Page::<Size2MiB>::containing_address(VirtAddr::new(0x30_0000));
        assert_eq!(page.start_address().as_u64(), 0x20_0000);
        assert_eq!(page.size(), 0x20_0000);
    }

    #[test]
    fn phys_frame_containing_address() {
        let frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(0x5678));
        assert_eq!(frame.start_address().as_u64(), 0x5000);
    }

    #[test]
    fn phys_frame_from_start_aligned() {
        let frame = PhysFrame::<Size4KiB>::from_start_address(PhysAddr::new(0x3000));
        assert!(frame.is_ok());
    }

    #[test]
    fn phys_frame_from_start_unaligned() {
        let frame = PhysFrame::<Size4KiB>::from_start_address(PhysAddr::new(0x3001));
        assert_eq!(frame.unwrap_err(), AddressNotAligned);
    }

    #[test]
    fn page_range_iterator() {
        let start = Page::<Size4KiB>::containing_address(VirtAddr::new(0x1000));
        let end = Page::<Size4KiB>::containing_address(VirtAddr::new(0x4000));
        let pages: Vec<_> = Page::range(start, end).collect();
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].start_address().as_u64(), 0x1000);
        assert_eq!(pages[1].start_address().as_u64(), 0x2000);
        assert_eq!(pages[2].start_address().as_u64(), 0x3000);
    }

    #[test]
    fn frame_range_iterator() {
        let start = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(0x0));
        let end = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(0x2000));
        let frames: Vec<_> = PhysFrame::range(start, end).collect();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].start_address().as_u64(), 0x0);
        assert_eq!(frames[1].start_address().as_u64(), 0x1000);
    }

    #[test]
    fn empty_range() {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(0x1000));
        let pages: Vec<_> = Page::range(page, page).collect();
        assert!(pages.is_empty());
    }
}
