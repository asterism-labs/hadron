//! A first-fit linked-list allocator over one fixed range of memory.
//!
//! Free blocks form a list sorted by address. Each free block stores its
//! size and the next block in its own first 16 bytes. Every block start
//! and size is a multiple of 16, so the padding a split leaves before or
//! after an allocation is either empty or big enough to hold a node.
//! Freeing a block merges it with the free blocks on either side.
//!
//! Allocation and freeing walk the list, so both cost O(free blocks).

use core::alloc::Layout;
use core::mem::{align_of, size_of};
use core::ptr::{self, NonNull};

/// A free block's header, stored in the block itself.
struct FreeBlock {
    size: usize,
    next: Option<NonNull<FreeBlock>>,
}

/// The smallest block, and the alignment of every block.
const BLOCK_ALIGN: usize = 16;

const _: () = assert!(size_of::<FreeBlock>() <= BLOCK_ALIGN);
const _: () = assert!(align_of::<FreeBlock>() <= BLOCK_ALIGN);

/// An allocator over `[start, end)`.
#[derive(Debug)]
pub struct LinkedListAllocator {
    /// The pointer the heap was made from. Block pointers are derived from
    /// it, so they keep its provenance.
    base: *mut u8,
    head: Option<NonNull<FreeBlock>>,
    start: usize,
    end: usize,
    free: usize,
}

// SAFETY: the heap owns its range, and the free list points only into it.
// Moving the heap to another CPU moves that ownership with it.
unsafe impl Send for LinkedListAllocator {}

impl LinkedListAllocator {
    /// An allocator over the 16-byte aligned part of `[start, start + size)`.
    ///
    /// # Safety
    ///
    /// The range must be writable memory that nothing else uses for as
    /// long as the heap or any allocation from it exists.
    #[must_use]
    pub unsafe fn new(start: *mut u8, size: usize) -> Self {
        let addr = start as usize;
        let begin = addr.next_multiple_of(BLOCK_ALIGN);
        let end = (addr + size) & !(BLOCK_ALIGN - 1);
        let mut heap = Self {
            base: start,
            head: None,
            start: begin,
            end: end.max(begin),
            free: 0,
        };
        if end > begin {
            // SAFETY: the caller hands over the range, and the block lies
            // inside it.
            unsafe { heap.insert(begin, end - begin) };
        }
        heap
    }

    /// Bytes not allocated, counting the padding between blocks.
    #[must_use]
    pub const fn free_bytes(&self) -> usize {
        self.free
    }

    /// Bytes the heap covers.
    #[must_use]
    pub const fn size(&self) -> usize {
        self.end - self.start
    }

    /// Number of free blocks. The heap is not fragmented when this is 1.
    #[must_use]
    pub fn free_blocks(&self) -> usize {
        let mut count = 0;
        let mut block = self.head;
        while let Some(b) = block {
            count += 1;
            // SAFETY: list nodes are free blocks inside the heap.
            block = unsafe { b.as_ref().next };
        }
        count
    }

    /// Memory for `layout`, or `None` if no free block fits.
    pub fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        let (size, align) = Self::block_layout(layout)?;
        let mut link: *mut Option<NonNull<FreeBlock>> = &raw mut self.head;
        // SAFETY: `link` starts at the list head and then points at the
        // `next` field of a free block inside the heap.
        while let Some(block) = unsafe { *link } {
            let addr = block.as_ptr() as usize;
            // SAFETY: `block` is a node of the list.
            let FreeBlock {
                size: block_size,
                next,
            } = unsafe { block.as_ptr().read() };
            let start = addr.next_multiple_of(align);
            let end = start.checked_add(size)?;
            if end > addr + block_size {
                // SAFETY: as above.
                link = unsafe { &raw mut (*block.as_ptr()).next };
                continue;
            }
            // Unlink the block, then put back the space before and after
            // the allocation. Both are multiples of 16.
            let mut rest = next;
            let back = addr + block_size - end;
            if back > 0 {
                // SAFETY: `[end, end + back)` is inside the free block.
                rest = Some(unsafe { self.write_node(end, back, rest) });
            }
            let front = start - addr;
            if front > 0 {
                // SAFETY: `[addr, start)` is inside the free block.
                rest = Some(unsafe { self.write_node(addr, front, rest) });
            }
            // SAFETY: as above.
            unsafe { *link = rest };
            self.free -= size;
            return NonNull::new(self.base.with_addr(start));
        }
        None
    }

    /// Returns memory to the heap.
    ///
    /// # Safety
    ///
    /// `ptr` must come from [`allocate`](Self::allocate) on this heap with
    /// the same `layout`, and must not be freed twice.
    ///
    /// # Panics
    ///
    /// Panics if `layout` is one [`allocate`](Self::allocate) refuses for
    /// overflowing, which breaks the safety contract.
    pub unsafe fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let (size, _) = Self::block_layout(layout).expect("the layout was allocated before");
        // SAFETY: the caller guarantees the block came from this heap and
        // is no longer in use.
        unsafe { self.insert(ptr.as_ptr() as usize, size) };
    }

    /// The block size and alignment for `layout`, or `None` if the size
    /// overflows.
    fn block_layout(layout: Layout) -> Option<(usize, usize)> {
        let size = layout.size().max(1).checked_next_multiple_of(BLOCK_ALIGN)?;
        Some((size, layout.align().max(BLOCK_ALIGN)))
    }

    /// Adds the free block `[addr, addr + size)` to the list in address
    /// order, merged with its neighbours.
    ///
    /// # Safety
    ///
    /// The block must be inside the heap, 16-byte aligned, a multiple of 16
    /// bytes, unused, and not overlap any free block.
    unsafe fn insert(&mut self, addr: usize, size: usize) {
        debug_assert!(addr >= self.start && addr + size <= self.end);
        self.free += size;
        let mut prev: Option<NonNull<FreeBlock>> = None;
        let mut next = self.head;
        while let Some(n) = next {
            if n.as_ptr() as usize > addr {
                break;
            }
            prev = Some(n);
            // SAFETY: list nodes are free blocks inside the heap.
            next = unsafe { n.as_ref().next };
        }

        let mut size = size;
        if let Some(n) = next
            && addr + size == n.as_ptr() as usize
        {
            // SAFETY: as above.
            let n = unsafe { n.as_ptr().read() };
            size += n.size;
            next = n.next;
        }
        if let Some(p) = prev {
            let p = p.as_ptr();
            // SAFETY: as above.
            let p_size = unsafe { (*p).size };
            if p as usize + p_size == addr {
                // SAFETY: as above.
                unsafe {
                    (*p).size = p_size + size;
                    (*p).next = next;
                }
                return;
            }
        }
        // SAFETY: the caller guarantees the block is unused and inside the
        // heap.
        let node = unsafe { self.write_node(addr, size, next) };
        match prev {
            // SAFETY: as above.
            Some(p) => unsafe { (*p.as_ptr()).next = Some(node) },
            None => self.head = Some(node),
        }
    }

    /// Writes a free block header at `addr`.
    ///
    /// # Safety
    ///
    /// `[addr, addr + size)` must be unused heap memory, 16-byte aligned.
    unsafe fn write_node(
        &self,
        addr: usize,
        size: usize,
        next: Option<NonNull<FreeBlock>>,
    ) -> NonNull<FreeBlock> {
        #[allow(clippy::cast_ptr_alignment)] // `addr` is 16-byte aligned
        let node = self.base.with_addr(addr).cast::<FreeBlock>();
        // SAFETY: the caller guarantees the memory is unused and aligned.
        unsafe { ptr::write(node, FreeBlock { size, next }) };
        // SAFETY: heap addresses are never 0.
        unsafe { NonNull::new_unchecked(node) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16-byte aligned buffer for a test heap.
    #[repr(C, align(4096))]
    struct Arena([u8; 64 * 1024]);

    fn arena() -> Box<Arena> {
        // SAFETY: all zero bytes are a valid `Arena`.
        unsafe { Box::<Arena>::new_zeroed().assume_init() }
    }

    fn heap(arena: &mut Arena) -> LinkedListAllocator {
        // SAFETY: the arena outlives the heap in every test.
        unsafe { LinkedListAllocator::new(arena.0.as_mut_ptr(), arena.0.len()) }
    }

    fn layout(size: usize, align: usize) -> Layout {
        Layout::from_size_align(size, align).unwrap()
    }

    #[test]
    fn starts_as_one_free_block() {
        let mut arena = arena();
        let heap = heap(&mut arena);
        assert_eq!(heap.free_bytes(), 64 * 1024);
        assert_eq!(heap.free_blocks(), 1);
    }

    #[test]
    fn unaligned_range_is_trimmed() {
        let mut arena = arena();
        // SAFETY: the arena outlives the heap.
        let heap = unsafe { LinkedListAllocator::new(arena.0.as_mut_ptr().add(3), 100) };
        // 3..103 trims to 16..96.
        assert_eq!(heap.size(), 80);
        assert_eq!(heap.free_bytes(), 80);
    }

    #[test]
    fn allocations_are_rounded_to_16_bytes() {
        let mut arena = arena();
        let mut heap = heap(&mut arena);
        let a = heap.allocate(layout(1, 1)).unwrap();
        let b = heap.allocate(layout(17, 1)).unwrap();
        assert_eq!(b.as_ptr() as usize - a.as_ptr() as usize, 16);
        assert_eq!(heap.free_bytes(), 64 * 1024 - 48);
    }

    #[test]
    fn honours_large_alignment_and_keeps_the_padding() {
        let mut arena = arena();
        let mut heap = heap(&mut arena);
        let _small = heap.allocate(layout(16, 16)).unwrap();
        let big = heap.allocate(layout(64, 4096)).unwrap();
        assert_eq!(big.as_ptr() as usize % 4096, 0);
        // The padding between the two is a free block again.
        assert_eq!(heap.free_blocks(), 2);
        assert_eq!(heap.free_bytes(), 64 * 1024 - 16 - 64);
    }

    #[test]
    fn freeing_merges_neighbours() {
        let mut arena = arena();
        let mut heap = heap(&mut arena);
        let blocks: Vec<_> = (0..4)
            .map(|_| heap.allocate(layout(32, 16)).unwrap())
            .collect();
        // SAFETY: each block came from this heap with this layout.
        unsafe {
            heap.deallocate(blocks[0], layout(32, 16));
            heap.deallocate(blocks[2], layout(32, 16));
        }
        assert_eq!(heap.free_blocks(), 3);
        // Freeing block 1 joins blocks 0, 1 and 2, and freeing 3 joins
        // the tail too.
        unsafe {
            heap.deallocate(blocks[1], layout(32, 16));
            heap.deallocate(blocks[3], layout(32, 16));
        }
        assert_eq!(heap.free_blocks(), 1);
        assert_eq!(heap.free_bytes(), 64 * 1024);
    }

    #[test]
    fn freed_memory_is_reused() {
        let mut arena = arena();
        let mut heap = heap(&mut arena);
        let a = heap.allocate(layout(128, 16)).unwrap();
        unsafe { heap.deallocate(a, layout(128, 16)) };
        assert_eq!(heap.allocate(layout(128, 16)), Some(a));
    }

    #[test]
    fn exhaustion_returns_none() {
        let mut arena = arena();
        let mut heap = heap(&mut arena);
        assert!(heap.allocate(layout(64 * 1024 + 1, 16)).is_none());
        let all = heap.allocate(layout(64 * 1024, 16)).unwrap();
        assert!(heap.allocate(layout(1, 1)).is_none());
        assert_eq!(heap.free_blocks(), 0);
        unsafe { heap.deallocate(all, layout(64 * 1024, 16)) };
        assert_eq!(heap.free_bytes(), 64 * 1024);
    }

    #[test]
    fn huge_requests_fail_instead_of_overflowing() {
        let mut arena = arena();
        let mut heap = heap(&mut arena);
        assert!(
            heap.allocate(layout(isize::MAX as usize - 64, 16))
                .is_none()
        );
    }

    /// Random allocations and frees, checked against a list of live
    /// blocks: each block is aligned, inside the heap, does not overlap
    /// another, and keeps the bytes written to it.
    #[test]
    #[allow(clippy::cast_possible_truncation)] // random numbers
    fn random_workload_keeps_blocks_disjoint_and_intact() {
        let mut arena = arena();
        let base = arena.0.as_ptr() as usize;
        let mut heap = heap(&mut arena);
        let mut rng = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let mut live: Vec<(NonNull<u8>, Layout, u8)> = Vec::new();
        for step in 0..20_000_u32 {
            let roll = next();
            if live.is_empty() || roll % 3 != 0 {
                let size = 1 + (next() % 700) as usize;
                let align = 1 << (next() % 9);
                let l = layout(size, align);
                let Some(ptr) = heap.allocate(l) else {
                    continue;
                };
                let addr = ptr.as_ptr() as usize;
                assert_eq!(addr % align, 0);
                assert!(addr >= base && addr + size <= base + 64 * 1024);
                for &(other, ol, _) in &live {
                    let o = other.as_ptr() as usize;
                    assert!(
                        addr + size <= o || o + ol.size() <= addr,
                        "overlap at step {step}"
                    );
                }
                let fill = step.to_le_bytes()[0];
                unsafe { ptr.as_ptr().write_bytes(fill, size) };
                live.push((ptr, l, fill));
            } else {
                let (ptr, l, fill) = live.swap_remove((next() as usize) % live.len());
                let bytes = unsafe { core::slice::from_raw_parts(ptr.as_ptr(), l.size()) };
                assert!(
                    bytes.iter().all(|&b| b == fill),
                    "block changed at step {step}"
                );
                unsafe { heap.deallocate(ptr, l) };
            }
        }
        for (ptr, l, _) in live.drain(..) {
            unsafe { heap.deallocate(ptr, l) };
        }
        assert_eq!(heap.free_bytes(), 64 * 1024);
        assert_eq!(heap.free_blocks(), 1);
    }
}
