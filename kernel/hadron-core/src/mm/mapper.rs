//! Architecture-independent page mapping interface.
//!
//! Provides [`MapFlags`], [`MapFlush`], [`PageMapper`], and [`PageTranslator`]
//! so that higher-level code (e.g. the VMM) can manipulate page tables without
//! knowing the underlying architecture.
//!
//! [`PageMapper<S>`] is parameterised by [`PageSize`]: an architecture
//! implements the trait for each page size it supports. [`PageTranslator`]
//! is separate because address translation is inherently page-size-agnostic.

use crate::addr::{PhysAddr, VirtAddr};
use crate::paging::{Page, PageSize, PhysFrame, Size4KiB};

bitflags::bitflags! {
    /// Architecture-independent page mapping flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MapFlags: u64 {
        /// Page is writable.
        const WRITABLE      = 1 << 0;
        /// Page is executable (if unset, no-execute is implied).
        const EXECUTABLE    = 1 << 1;
        /// Page is accessible from user mode.
        const USER          = 1 << 2;
        /// Global page (not flushed on address-space switch).
        const GLOBAL        = 1 << 3;
        /// Caching disabled for this page.
        const CACHE_DISABLE = 1 << 4;
    }
}

/// Error from unmap / update_flags operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmapError {
    /// The page is not mapped.
    NotMapped,
    /// The entry maps a different page size than requested.
    ///
    /// For example, attempting to unmap a 4 KiB page within a 2 MiB huge page,
    /// or vice versa.
    SizeMismatch,
}

/// A pending TLB flush for a single page.
///
/// Created by page table modification operations. Flushes the TLB entry
/// on drop unless [`.flush()`](Self::flush) or [`.ignore()`](Self::ignore)
/// is called first.
#[must_use = "TLB flush is pending; call .flush() or .ignore()"]
pub struct MapFlush {
    virt: VirtAddr,
    needs_flush: bool,
}

impl MapFlush {
    /// Creates a new pending flush for the given virtual address.
    pub(crate) fn new(virt: VirtAddr) -> Self {
        Self {
            virt,
            needs_flush: true,
        }
    }

    /// Flush the TLB entry immediately.
    pub fn flush(mut self) {
        self.needs_flush = false;
        arch_flush_page(self.virt);
    }

    /// Explicitly opt out of flushing (e.g. fresh mappings not yet in TLB,
    /// or batch flushes handled separately).
    pub fn ignore(mut self) {
        self.needs_flush = false;
    }
}

impl Drop for MapFlush {
    fn drop(&mut self) {
        if self.needs_flush {
            arch_flush_page(self.virt);
        }
    }
}

/// Architecture-dispatched single-page TLB flush.
#[inline]
fn arch_flush_page(virt: VirtAddr) {
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::instructions::tlb::flush(virt);

    #[cfg(target_arch = "aarch64")]
    {
        let _ = virt; // TODO: aarch64 TLB flush
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapflags_default_empty() {
        let flags = MapFlags::empty();
        assert!(flags.is_empty());
        assert_eq!(flags.bits(), 0);
    }

    #[test]
    fn mapflags_combination() {
        let flags = MapFlags::WRITABLE | MapFlags::USER;
        assert!(flags.contains(MapFlags::WRITABLE));
        assert!(flags.contains(MapFlags::USER));
        assert!(!flags.contains(MapFlags::EXECUTABLE));
    }

    #[test]
    fn mapflags_all_bits_distinct() {
        let all = [
            MapFlags::WRITABLE,
            MapFlags::EXECUTABLE,
            MapFlags::USER,
            MapFlags::GLOBAL,
            MapFlags::CACHE_DISABLE,
        ];
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                if i != j {
                    assert!((*a & *b).is_empty(), "{a:?} and {b:?} share bits");
                }
            }
        }
    }

    #[test]
    fn unmap_error_variants() {
        assert_ne!(UnmapError::NotMapped, UnmapError::SizeMismatch);
    }
}

/// Architecture-independent page table mapping interface, generic over page size.
///
/// An architecture implements this trait for each page size it supports.
/// For example, x86_64 implements `PageMapper<Size4KiB>`, `PageMapper<Size2MiB>`,
/// and `PageMapper<Size1GiB>`.
///
/// # Safety
///
/// Implementations must correctly manipulate hardware page tables for the
/// given page size.
pub unsafe trait PageMapper<S: PageSize> {
    /// Maps a virtual page to a physical frame with the given flags.
    ///
    /// Allocates intermediate page table frames (always 4 KiB) as needed.
    ///
    /// Returns a [`MapFlush`] that the caller must either `.flush()` or
    /// `.ignore()`. Dropping the `MapFlush` without calling either will
    /// flush automatically.
    ///
    /// # Safety
    ///
    /// - `root` must point to a valid root page table.
    /// - `alloc` must return zeroed 4 KiB frames.
    unsafe fn map(
        &self,
        root: PhysAddr,
        page: Page<S>,
        frame: PhysFrame<S>,
        flags: MapFlags,
        alloc: &mut dyn FnMut() -> PhysFrame<Size4KiB>,
    ) -> MapFlush;

    /// Unmaps a page and returns the physical frame that was mapped,
    /// along with a [`MapFlush`] for TLB invalidation.
    ///
    /// Returns [`UnmapError::SizeMismatch`] if the entry at this address
    /// maps a different page size than `S`.
    ///
    /// # Safety
    ///
    /// `root` must point to a valid root page table.
    unsafe fn unmap(
        &self,
        root: PhysAddr,
        page: Page<S>,
    ) -> Result<(PhysFrame<S>, MapFlush), UnmapError>;

    /// Updates the flags of a mapped page.
    ///
    /// Returns a [`MapFlush`] for TLB invalidation.
    ///
    /// Returns [`UnmapError::SizeMismatch`] if the entry at this address
    /// maps a different page size than `S`.
    ///
    /// # Safety
    ///
    /// `root` must point to a valid root page table.
    unsafe fn update_flags(
        &self,
        root: PhysAddr,
        page: Page<S>,
        flags: MapFlags,
    ) -> Result<MapFlush, UnmapError>;
}

/// Architecture-independent virtual address translation.
///
/// Separated from [`PageMapper`] because translation is inherently
/// page-size-agnostic: the implementation walks the page table and detects
/// the mapping size dynamically.
///
/// # Safety
///
/// Implementations must correctly walk hardware page tables.
pub unsafe trait PageTranslator {
    /// Translates a virtual address to physical.
    ///
    /// Returns `None` if the address is not mapped. Handles all page sizes
    /// internally.
    ///
    /// # Safety
    ///
    /// `root` must point to a valid root page table.
    unsafe fn translate_addr(&self, root: PhysAddr, virt: VirtAddr) -> Option<PhysAddr>;
}
