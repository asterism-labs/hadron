//! Physical frames and the trait that hands them out.

use core::fmt;

use arch_x86_64::PhysAddr;
use arch_x86_64::structures::paging::PAGE_SIZE;

/// A 4 KiB aligned, 4 KiB physical frame.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysFrame(PhysAddr);

impl PhysFrame {
    /// The frame starting at `start`, or `None` if `start` is not 4 KiB
    /// aligned.
    #[must_use]
    pub const fn from_start(start: PhysAddr) -> Option<Self> {
        if start.is_aligned(PAGE_SIZE) {
            Some(Self(start))
        } else {
            None
        }
    }

    /// The frame that holds `addr`.
    #[must_use]
    pub const fn containing(addr: PhysAddr) -> Self {
        Self(addr.align_down(PAGE_SIZE))
    }

    /// The frame with number `number`, which starts at `number * 4096`.
    ///
    /// # Panics
    ///
    /// Panics if the frame lies above the 52-bit physical address limit.
    #[must_use]
    pub const fn from_number(number: u64) -> Self {
        Self(PhysAddr::new(number * PAGE_SIZE))
    }

    /// The first byte of the frame.
    #[must_use]
    pub const fn start(self) -> PhysAddr {
        self.0
    }

    /// The frame's index in physical memory: its address divided by 4096.
    #[must_use]
    pub const fn number(self) -> u64 {
        self.0.as_u64() / PAGE_SIZE
    }
}

impl fmt::Debug for PhysFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysFrame({:#x})", self.0)
    }
}

/// Something that hands out unused 4 KiB frames.
///
/// The frames are not zeroed.
pub trait FrameAllocator {
    /// An unused frame, or `None` if none are left.
    fn allocate_frame(&mut self) -> Option<PhysFrame>;
}
