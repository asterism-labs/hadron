//! The direct map: every physical address `p` is mapped at `p + offset`.

use arch_x86_64::structures::paging::PageTable;
use arch_x86_64::{PhysAddr, VirtAddr};

/// A direct map of physical memory at a fixed offset.
///
/// During boot the kernel first uses Limine's direct map and then its own,
/// at a different offset. Code that keeps physical addresses and converts
/// them through the current `DirectMap` keeps working across the switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectMap {
    offset: u64,
}

impl DirectMap {
    /// A direct map at `offset`.
    #[must_use]
    pub const fn new(offset: u64) -> Self {
        Self { offset }
    }

    /// The virtual address of physical address 0.
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// The virtual address `addr` is mapped at.
    ///
    /// # Panics
    ///
    /// Panics if the result is not canonical, which means `addr` is past
    /// the end of the direct map.
    #[must_use]
    pub fn phys_to_virt(self, addr: PhysAddr) -> VirtAddr {
        VirtAddr::new(self.offset + addr.as_u64())
    }

    /// A pointer to the `T` at physical address `addr`.
    #[must_use]
    pub fn ptr<T>(self, addr: PhysAddr) -> *mut T {
        self.phys_to_virt(addr).as_mut_ptr()
    }

    /// A pointer to the page table in the frame at `addr`.
    #[must_use]
    pub fn table(self, addr: PhysAddr) -> *mut PageTable {
        self.ptr(addr)
    }

    /// Writes `value` into `len` consecutive `T`s starting at `addr` and
    /// returns them as a slice.
    ///
    /// # Safety
    ///
    /// The `len` `T`s at `addr` must be mapped through this direct map for
    /// `'a`, aligned for `T`, and used by nothing else for `'a`.
    #[must_use]
    pub unsafe fn init_slice<'a, T: Copy>(
        self,
        addr: PhysAddr,
        len: usize,
        value: T,
    ) -> &'a mut [T] {
        let ptr = self.ptr::<T>(addr);
        for i in 0..len {
            // SAFETY: the caller guarantees the memory is mapped, aligned
            // and unused, and `i < len`.
            unsafe { ptr.add(i).write(value) };
        }
        // SAFETY: every element was just written, and the caller gives
        // this slice the only access for `'a`.
        unsafe { core::slice::from_raw_parts_mut(ptr, len) }
    }
}
