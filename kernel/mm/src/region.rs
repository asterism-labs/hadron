//! Virtual address region allocators.
//!
//! Provides [`RegionAllocator`] (bump-only, used for the kernel heap) and
//! [`FreeRegionAllocator`] (bump + sorted free-list with coalescing, used
//! for stacks and MMIO regions that support deallocation).

use core::fmt;

use planck_noalloc::vec::ArrayVec;

use crate::layout::VirtRegion;
use hadron_core::addr::VirtAddr;

/// Page-align `size` upward (round to next 4 KiB boundary).
#[inline]
fn page_align_up(size: u64) -> u64 {
    (size + super::PAGE_MASK as u64) & !(super::PAGE_MASK as u64)
}

// ---------------------------------------------------------------------------
// RegionAllocator (bump-only)
// ---------------------------------------------------------------------------

/// A simple bump allocator for virtual address ranges.
///
/// Allocates contiguous ranges by advancing a cursor. Does not support
/// deallocation. Used for the kernel heap region.
#[derive(Debug)]
pub struct RegionAllocator {
    region: VirtRegion,
    next: u64,
}

impl RegionAllocator {
    /// Creates a new allocator covering the given virtual region.
    pub fn new(region: VirtRegion) -> Self {
        Self {
            next: region.base().as_u64(),
            region,
        }
    }

    /// Allocates `size` bytes (rounded up to page alignment) from the region.
    /// Returns the base address of the allocated range, or `None` if the
    /// region is exhausted.
    pub fn allocate(&mut self, size: u64) -> Option<VirtAddr> {
        let aligned_size = page_align_up(size);
        let end = self.next + aligned_size;
        let region_end = self.region.base().as_u64() + self.region.max_size();

        if end > region_end {
            return None;
        }

        let base = self.next;
        self.next = end;
        Some(VirtAddr::new_truncate(base))
    }

    /// Returns the next allocation address (current watermark).
    pub fn current(&self) -> VirtAddr {
        VirtAddr::new_truncate(self.next)
    }

    /// Returns the number of bytes already allocated.
    pub fn used(&self) -> u64 {
        self.next - self.region.base().as_u64()
    }
}

// ---------------------------------------------------------------------------
// FreeRegionAllocator (bump + free-list with coalescing)
// ---------------------------------------------------------------------------

/// Errors from the [`FreeRegionAllocator`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionAllocError {
    /// The free list is at capacity and the range could not be coalesced
    /// with an existing entry.
    FreeListFull,
}

impl fmt::Display for RegionAllocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FreeListFull => write!(f, "region allocator free list full"),
        }
    }
}

/// A contiguous free virtual address range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FreeRange {
    /// Base virtual address (page-aligned).
    base: u64,
    /// Size in bytes (page-aligned, > 0).
    size: u64,
}

/// A virtual address region allocator with deallocation and coalescing support.
///
/// Maintains a sorted array of free ranges within a [`VirtRegion`]. Supports
/// first-fit allocation and deallocation with immediate neighbor coalescing.
/// Uses a fixed-capacity [`ArrayVec`] for no-alloc operation.
///
/// # Algorithm
///
/// - **Allocate**: scan free-list for first-fit; fall back to bumping watermark.
/// - **Deallocate**: binary-search for insertion point, coalesce with neighbors,
///   retract watermark if the freed range is at the tail.
#[derive(Debug)]
pub struct FreeRegionAllocator<const N: usize> {
    region: VirtRegion,
    /// Free ranges, sorted by base address. Adjacent ranges are always coalesced.
    free_list: ArrayVec<FreeRange, N>,
    /// High-water mark: the next address that the bump allocator would hand out.
    watermark: u64,
}

impl<const N: usize> FreeRegionAllocator<N> {
    /// Creates a new allocator covering the given virtual region.
    /// The entire region starts as unallocated (watermark at base).
    pub fn new(region: VirtRegion) -> Self {
        Self {
            watermark: region.base().as_u64(),
            free_list: ArrayVec::new(),
            region,
        }
    }

    /// Allocates `size` bytes (rounded up to page alignment) from the region.
    ///
    /// First tries to find a suitable range in the free list (first-fit).
    /// Falls back to bumping the watermark if no free range is large enough.
    /// Returns `None` if the region is exhausted.
    pub fn allocate(&mut self, size: u64) -> Option<VirtAddr> {
        let aligned_size = page_align_up(size);
        if aligned_size == 0 {
            return Some(VirtAddr::new_truncate(self.watermark));
        }

        // First-fit scan of free list.
        for i in 0..self.free_list.len() {
            let entry = self.free_list[i];
            if entry.size >= aligned_size {
                let base = entry.base;
                if entry.size == aligned_size {
                    self.free_list.remove(i);
                } else {
                    self.free_list[i] = FreeRange {
                        base: entry.base + aligned_size,
                        size: entry.size - aligned_size,
                    };
                }
                return Some(VirtAddr::new_truncate(base));
            }
        }

        // Fall back to bump allocation.
        let end = self.watermark + aligned_size;
        let region_end = self.region.base().as_u64() + self.region.max_size();
        if end > region_end {
            return None;
        }

        let base = self.watermark;
        self.watermark = end;
        Some(VirtAddr::new_truncate(base))
    }

    /// Returns a previously allocated range to the allocator.
    ///
    /// `addr` must be the exact base returned by [`allocate`](Self::allocate),
    /// and `size` must match the original request (page-rounding is re-applied).
    ///
    /// # Errors
    ///
    /// Returns [`RegionAllocError::FreeListFull`] if the free list is at
    /// capacity and the range cannot be coalesced with an existing entry.
    ///
    /// # Panics
    ///
    /// Debug-panics if the range is outside the region or overlaps with
    /// existing free ranges (double-free detection).
    pub fn deallocate(&mut self, addr: VirtAddr, size: u64) -> Result<(), RegionAllocError> {
        let base = addr.as_u64();
        let aligned_size = page_align_up(size);
        if aligned_size == 0 {
            return Ok(());
        }
        let range_end = base + aligned_size;

        debug_assert!(
            base >= self.region.base().as_u64(),
            "deallocate: address below region base"
        );
        debug_assert!(
            range_end <= self.region.base().as_u64() + self.region.max_size(),
            "deallocate: address above region end"
        );
        debug_assert!(
            range_end <= self.watermark,
            "deallocate: address beyond watermark (never allocated)"
        );

        // Watermark retraction: if the range abuts the watermark, just pull it back.
        if range_end == self.watermark {
            self.watermark = base;
            // Continue retracting if the last free-list entry now abuts the watermark.
            self.retract_watermark();
            return Ok(());
        }

        // Binary search for insertion point: first entry with base > addr.
        let idx = self.insertion_index(base);

        // Debug: check no overlap with neighbors.
        debug_assert!(
            idx == 0 || {
                let prev = self.free_list[idx - 1];
                prev.base + prev.size <= base
            },
            "deallocate: overlaps with predecessor (double-free?)"
        );
        debug_assert!(
            idx >= self.free_list.len() || self.free_list[idx].base >= range_end,
            "deallocate: overlaps with successor (double-free?)"
        );

        // Try to coalesce with predecessor and/or successor.
        let merge_prev = idx > 0 && {
            let prev = self.free_list[idx - 1];
            prev.base + prev.size == base
        };
        let merge_next = idx < self.free_list.len() && self.free_list[idx].base == range_end;

        match (merge_prev, merge_next) {
            (true, true) => {
                // Merge predecessor + freed range + successor into predecessor.
                let succ = self.free_list.remove(idx);
                self.free_list[idx - 1].size += aligned_size + succ.size;
            }
            (true, false) => {
                // Extend predecessor.
                self.free_list[idx - 1].size += aligned_size;
            }
            (false, true) => {
                // Extend successor backward.
                self.free_list[idx].base = base;
                self.free_list[idx].size += aligned_size;
            }
            (false, false) => {
                // No coalescing possible: insert a new entry.
                if self.free_list.is_full() {
                    return Err(RegionAllocError::FreeListFull);
                }
                self.free_list.insert(
                    idx,
                    FreeRange {
                        base,
                        size: aligned_size,
                    },
                );
            }
        }

        // Retract watermark if trailing free entry now abuts it.
        self.retract_watermark();
        Ok(())
    }

    /// Returns the current watermark (highest address ever bumped to, minus retractions).
    pub fn watermark(&self) -> VirtAddr {
        VirtAddr::new_truncate(self.watermark)
    }

    /// Returns the number of bytes between region base and watermark.
    pub fn watermark_used(&self) -> u64 {
        self.watermark - self.region.base().as_u64()
    }

    /// Returns the number of entries in the free list.
    pub fn free_list_len(&self) -> usize {
        self.free_list.len()
    }

    /// Returns the total bytes currently in the free list.
    pub fn free_bytes(&self) -> u64 {
        self.free_list.iter().map(|r| r.size).sum()
    }

    /// Binary search: returns index of the first entry with `base > addr`.
    fn insertion_index(&self, addr: u64) -> usize {
        let slice = self.free_list.as_slice();
        match slice.binary_search_by_key(&addr, |r| r.base) {
            Ok(i) => i,  // exact match (shouldn't happen: would mean double-free)
            Err(i) => i, // insertion point
        }
    }

    /// Retract the watermark as long as the last free-list entry abuts it.
    fn retract_watermark(&mut self) {
        while let Some(last) = self.free_list.last() {
            if last.base + last.size == self.watermark {
                self.watermark = last.base;
                let _ = self.free_list.pop();
            } else {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_region(base: u64, size: u64) -> RegionAllocator {
        let region = VirtRegion::new(VirtAddr::new_truncate(base), size);
        RegionAllocator::new(region)
    }

    fn test_free_region<const N: usize>(base: u64, size: u64) -> FreeRegionAllocator<N> {
        let region = VirtRegion::new(VirtAddr::new_truncate(base), size);
        FreeRegionAllocator::new(region)
    }

    // --- RegionAllocator tests ---

    #[test]
    fn initial_state() {
        let alloc = test_region(0x1000, 0x10000);
        assert_eq!(alloc.current().as_u64(), 0x1000);
        assert_eq!(alloc.used(), 0);
    }

    #[test]
    fn allocate_single_page() {
        let mut alloc = test_region(0x1000, 0x10000);
        let addr = alloc.allocate(0x1000);
        assert_eq!(addr.unwrap().as_u64(), 0x1000);
        assert_eq!(alloc.used(), 0x1000);
    }

    #[test]
    fn allocate_rounds_up_to_page() {
        let mut alloc = test_region(0x1000, 0x10000);
        let addr = alloc.allocate(1);
        assert_eq!(addr.unwrap().as_u64(), 0x1000);
        // 1 byte rounds up to 4KiB (0x1000).
        assert_eq!(alloc.used(), 0x1000);
    }

    #[test]
    fn allocate_sequential() {
        let mut alloc = test_region(0x1000, 0x10000);
        let a = alloc.allocate(0x1000).unwrap();
        let b = alloc.allocate(0x1000).unwrap();
        assert_eq!(a.as_u64(), 0x1000);
        assert_eq!(b.as_u64(), 0x2000);
    }

    #[test]
    fn allocate_exhausts_region() {
        let mut alloc = test_region(0x1000, 0x2000);
        let a = alloc.allocate(0x1000);
        assert!(a.is_some());
        let b = alloc.allocate(0x1000);
        assert!(b.is_some());
        let c = alloc.allocate(0x1000);
        assert!(c.is_none());
    }

    #[test]
    fn allocate_too_large() {
        let mut alloc = test_region(0x1000, 0x2000);
        let result = alloc.allocate(0x3000);
        assert!(result.is_none());
    }

    #[test]
    fn allocate_zero_bytes() {
        let mut alloc = test_region(0x1000, 0x10000);
        let addr = alloc.allocate(0);
        // (0 + 0xFFF) & !0xFFF = 0, so next is unchanged.
        assert_eq!(addr.unwrap().as_u64(), 0x1000);
        assert_eq!(alloc.used(), 0);
    }

    #[test]
    fn watermark_tracks_rounded_size() {
        let mut alloc = test_region(0x1000, 0x100000);
        alloc.allocate(0x5001).unwrap();
        // 0x5001 rounds up to 0x6000.
        assert_eq!(alloc.used(), 0x6000);
    }

    // --- FreeRegionAllocator tests ---

    #[test]
    fn free_alloc_initial_state() {
        let alloc = test_free_region::<16>(0x1000, 0x10000);
        assert_eq!(alloc.watermark().as_u64(), 0x1000);
        assert_eq!(alloc.watermark_used(), 0);
        assert_eq!(alloc.free_list_len(), 0);
        assert_eq!(alloc.free_bytes(), 0);
    }

    #[test]
    fn free_alloc_basic_bump() {
        let mut alloc = test_free_region::<16>(0x1000, 0x10000);
        let a = alloc.allocate(0x1000).unwrap();
        assert_eq!(a.as_u64(), 0x1000);
        let b = alloc.allocate(0x1000).unwrap();
        assert_eq!(b.as_u64(), 0x2000);
        assert_eq!(alloc.watermark_used(), 0x2000);
    }

    #[test]
    fn free_alloc_dealloc_retract_watermark() {
        let mut alloc = test_free_region::<16>(0x1000, 0x10000);
        let a = alloc.allocate(0x1000).unwrap();
        let b = alloc.allocate(0x1000).unwrap();
        assert_eq!(b.as_u64(), 0x2000);

        // Free the last allocation -- watermark should retract.
        alloc.deallocate(b, 0x1000).unwrap();
        assert_eq!(alloc.watermark().as_u64(), 0x2000);
        assert_eq!(alloc.free_list_len(), 0);

        // Free the first allocation -- watermark should retract further.
        alloc.deallocate(a, 0x1000).unwrap();
        assert_eq!(alloc.watermark().as_u64(), 0x1000);
        assert_eq!(alloc.free_list_len(), 0);
    }

    #[test]
    fn free_alloc_reuse_freed_range() {
        let mut alloc = test_free_region::<16>(0x1000, 0x10000);
        let a = alloc.allocate(0x1000).unwrap();
        let _b = alloc.allocate(0x1000).unwrap();
        let _c = alloc.allocate(0x1000).unwrap();

        // Free the first allocation (creates a hole in the free list).
        alloc.deallocate(a, 0x1000).unwrap();
        assert_eq!(alloc.free_list_len(), 1);

        // Allocate again -- should reuse the freed range.
        let d = alloc.allocate(0x1000).unwrap();
        assert_eq!(d.as_u64(), a.as_u64());
        assert_eq!(alloc.free_list_len(), 0);
    }

    #[test]
    fn free_alloc_coalesce_predecessor() {
        let mut alloc = test_free_region::<16>(0x1000, 0x10000);
        let a = alloc.allocate(0x1000).unwrap();
        let b = alloc.allocate(0x1000).unwrap();
        let _c = alloc.allocate(0x1000).unwrap();

        alloc.deallocate(a, 0x1000).unwrap();
        assert_eq!(alloc.free_list_len(), 1);

        // Free b -- should coalesce with a.
        alloc.deallocate(b, 0x1000).unwrap();
        assert_eq!(alloc.free_list_len(), 1);
        assert_eq!(alloc.free_bytes(), 0x2000);
    }

    #[test]
    fn free_alloc_coalesce_successor() {
        let mut alloc = test_free_region::<16>(0x1000, 0x10000);
        let a = alloc.allocate(0x1000).unwrap();
        let b = alloc.allocate(0x1000).unwrap();
        let _c = alloc.allocate(0x1000).unwrap();

        alloc.deallocate(b, 0x1000).unwrap();
        assert_eq!(alloc.free_list_len(), 1);

        // Free a -- should coalesce with b.
        alloc.deallocate(a, 0x1000).unwrap();
        assert_eq!(alloc.free_list_len(), 1);
        assert_eq!(alloc.free_bytes(), 0x2000);
    }

    #[test]
    fn free_alloc_coalesce_both() {
        let mut alloc = test_free_region::<16>(0x1000, 0x10000);
        let a = alloc.allocate(0x1000).unwrap();
        let b = alloc.allocate(0x1000).unwrap();
        let c = alloc.allocate(0x1000).unwrap();
        let _d = alloc.allocate(0x1000).unwrap();

        alloc.deallocate(a, 0x1000).unwrap();
        alloc.deallocate(c, 0x1000).unwrap();
        assert_eq!(alloc.free_list_len(), 2);

        // Free b -- should coalesce with both a and c.
        alloc.deallocate(b, 0x1000).unwrap();
        assert_eq!(alloc.free_list_len(), 1);
        assert_eq!(alloc.free_bytes(), 0x3000);
    }

    #[test]
    fn free_alloc_watermark_retraction_chain() {
        let mut alloc = test_free_region::<16>(0x1000, 0x10000);
        let a = alloc.allocate(0x1000).unwrap();
        let b = alloc.allocate(0x1000).unwrap();
        let c = alloc.allocate(0x1000).unwrap();

        // Free b and c (in that order) — b goes to free list, c retracts watermark.
        alloc.deallocate(b, 0x1000).unwrap();
        alloc.deallocate(c, 0x1000).unwrap();
        // c retraction should pull watermark to b.base+b.size=0x3000,
        // then b is at the tail and retracts to 0x2000.
        assert_eq!(alloc.watermark().as_u64(), 0x2000);
        assert_eq!(alloc.free_list_len(), 0);

        // Free a — retracts all the way.
        alloc.deallocate(a, 0x1000).unwrap();
        assert_eq!(alloc.watermark().as_u64(), 0x1000);
    }

    #[test]
    fn free_alloc_region_exhausted() {
        let mut alloc = test_free_region::<16>(0x1000, 0x2000);
        alloc.allocate(0x1000).unwrap();
        alloc.allocate(0x1000).unwrap();
        assert!(alloc.allocate(0x1000).is_none());
    }

    #[test]
    fn free_alloc_first_fit_partial() {
        let mut alloc = test_free_region::<16>(0x1000, 0x10000);
        let a = alloc.allocate(0x3000).unwrap(); // 3 pages
        let _b = alloc.allocate(0x1000).unwrap();

        // Free a -- creates a 3-page hole.
        alloc.deallocate(a, 0x3000).unwrap();

        // Allocate 1 page -- should split the free range.
        let c = alloc.allocate(0x1000).unwrap();
        assert_eq!(c.as_u64(), a.as_u64());
        assert_eq!(alloc.free_list_len(), 1);
        assert_eq!(alloc.free_bytes(), 0x2000);
    }

    #[test]
    fn free_alloc_free_list_full_error() {
        // Capacity of 2: can hold at most 2 disjoint free ranges.
        let mut alloc = test_free_region::<2>(0x1000, 0x10000);
        let a = alloc.allocate(0x1000).unwrap();
        let _b = alloc.allocate(0x1000).unwrap();
        let c = alloc.allocate(0x1000).unwrap();
        let _d = alloc.allocate(0x1000).unwrap();
        let e = alloc.allocate(0x1000).unwrap();
        let _f = alloc.allocate(0x1000).unwrap();

        alloc.deallocate(a, 0x1000).unwrap();
        alloc.deallocate(c, 0x1000).unwrap();
        assert_eq!(alloc.free_list_len(), 2);

        // Third non-adjacent free should fail.
        let result = alloc.deallocate(e, 0x1000);
        assert_eq!(result, Err(RegionAllocError::FreeListFull));
    }

    #[test]
    fn free_alloc_zero_size() {
        let mut alloc = test_free_region::<16>(0x1000, 0x10000);
        let a = alloc.allocate(0).unwrap();
        assert_eq!(a.as_u64(), 0x1000);
        assert_eq!(alloc.watermark_used(), 0);
    }

    #[test]
    fn free_alloc_full_cycle() {
        // Allocate all, free all, allocate again.
        let mut alloc = test_free_region::<16>(0x1000, 0x4000);
        let a = alloc.allocate(0x1000).unwrap();
        let b = alloc.allocate(0x1000).unwrap();
        let c = alloc.allocate(0x1000).unwrap();
        let d = alloc.allocate(0x1000).unwrap();
        assert!(alloc.allocate(0x1000).is_none());

        // Free in reverse order — all retract watermark.
        alloc.deallocate(d, 0x1000).unwrap();
        alloc.deallocate(c, 0x1000).unwrap();
        alloc.deallocate(b, 0x1000).unwrap();
        alloc.deallocate(a, 0x1000).unwrap();
        assert_eq!(alloc.watermark().as_u64(), 0x1000);
        assert_eq!(alloc.free_list_len(), 0);

        // Should be able to allocate all 4 pages again.
        assert!(alloc.allocate(0x1000).is_some());
        assert!(alloc.allocate(0x1000).is_some());
        assert!(alloc.allocate(0x1000).is_some());
        assert!(alloc.allocate(0x1000).is_some());
        assert!(alloc.allocate(0x1000).is_none());
    }
}
