//! Kernel virtual address space layout.
//!
//! Defines the [`MemoryLayout`] describing where kernel regions (heap, stacks,
//! MMIO, per-CPU, vDSO) live in the virtual address space. The layout is
//! KASLR-ready: all regions are defined as const offsets from a runtime
//! `regions_base` that can be randomized.

use crate::addr::VirtAddr;

/// Default base address for kernel regions (non-KASLR).
pub const DEFAULT_REGIONS_BASE: u64 = 0xFFFF_C000_0000_0000;

/// Offset from regions_base to the heap region.
pub const HEAP_OFFSET: u64 = 0;
/// Maximum heap size: 2 TiB.
pub const HEAP_MAX_SIZE: u64 = 2 * 1024 * 1024 * 1024 * 1024;

/// Offset from regions_base to the kernel stacks region.
pub const STACKS_OFFSET: u64 = 8 * 1024 * 1024 * 1024 * 1024; // +8 TiB
/// Maximum stacks region size: 512 GiB.
pub const STACKS_MAX_SIZE: u64 = 512 * 1024 * 1024 * 1024;

/// Offset from regions_base to the MMIO region.
pub const MMIO_OFFSET: u64 = 16 * 1024 * 1024 * 1024 * 1024; // +16 TiB
/// Maximum MMIO region size: 1 TiB.
pub const MMIO_MAX_SIZE: u64 = 1024 * 1024 * 1024 * 1024;

/// Offset from regions_base to the per-CPU region.
pub const PERCPU_OFFSET: u64 = 32 * 1024 * 1024 * 1024 * 1024; // +32 TiB
/// Maximum per-CPU region size: 1 TiB.
pub const PERCPU_MAX_SIZE: u64 = 1024 * 1024 * 1024 * 1024;

/// Offset from regions_base to the vDSO region.
pub const VDSO_OFFSET: u64 = 48 * 1024 * 1024 * 1024 * 1024; // +48 TiB
/// Maximum vDSO region size: 2 MiB.
pub const VDSO_MAX_SIZE: u64 = 2 * 1024 * 1024;

/// Fixed kernel image base address (not KASLR-shifted).
pub const KERNEL_IMAGE_BASE: u64 = 0xFFFF_FFFF_8000_0000;
/// Maximum kernel image size: 128 MiB.
pub const KERNEL_IMAGE_MAX_SIZE: u64 = 128 * 1024 * 1024;

/// Initial heap size: 4 MiB.
pub const INITIAL_HEAP_SIZE: u64 = 4 * 1024 * 1024;
/// Minimum heap growth increment: 64 KiB.
pub const HEAP_GROW_MIN: u64 = 64 * 1024;

/// A virtual address region with a base and maximum size.
#[derive(Debug, Clone, Copy)]
pub struct VirtRegion {
    base: VirtAddr,
    max_size: u64,
}

impl VirtRegion {
    /// Creates a new virtual region.
    pub const fn new(base: VirtAddr, max_size: u64) -> Self {
        Self { base, max_size }
    }

    /// Returns the base address of this region.
    #[inline]
    pub const fn base(&self) -> VirtAddr {
        self.base
    }

    /// Returns the maximum size of this region.
    #[inline]
    pub const fn max_size(&self) -> u64 {
        self.max_size
    }

    /// Returns the end address (base + max_size).
    #[inline]
    pub fn end(&self) -> VirtAddr {
        self.base + self.max_size
    }

    /// Returns true if `addr` falls within this region.
    #[inline]
    pub fn contains(&self, addr: VirtAddr) -> bool {
        addr.as_u64() >= self.base.as_u64()
            && addr.as_u64() < self.base.as_u64() + self.max_size
    }
}

/// Describes the kernel's virtual address space layout.
#[derive(Debug, Clone, Copy)]
pub struct MemoryLayout {
    /// HHDM base (provided by bootloader, not KASLR-shifted).
    pub hhdm_base: VirtAddr,
    /// Size of the HHDM region (covers all physical memory).
    pub hhdm_size: u64,
    /// Base address for KASLR-randomizable regions.
    pub regions_base: VirtAddr,
    /// Kernel heap region.
    pub heap: VirtRegion,
    /// Kernel stack region.
    pub stacks: VirtRegion,
    /// MMIO mapping region.
    pub mmio: VirtRegion,
    /// Per-CPU data region.
    pub percpu: VirtRegion,
    /// vDSO/VVAR region.
    pub vdso: VirtRegion,
    /// Kernel image region (fixed, not KASLR-shifted).
    pub kernel_image: VirtRegion,
}

/// Identifies which kernel virtual address region a faulting address belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultRegion {
    /// Kernel heap.
    Heap,
    /// Kernel stacks (includes guard pages).
    Stacks,
    /// Memory-mapped I/O.
    Mmio,
    /// Per-CPU data.
    PerCpu,
    /// Kernel image (.text, .rodata, .data, .bss).
    KernelImage,
    /// Higher-half direct map.
    Hhdm,
    /// Address does not belong to any known region.
    Unknown,
}

impl MemoryLayout {
    /// Creates a new `MemoryLayout` from the HHDM offset and maximum physical
    /// address. Uses the default (non-KASLR) regions base.
    pub fn new(hhdm_offset: u64, max_phys: u64) -> Self {
        Self::with_regions_base(hhdm_offset, max_phys, DEFAULT_REGIONS_BASE)
    }

    /// Creates a new `MemoryLayout` with a custom regions base (for KASLR).
    pub fn with_regions_base(hhdm_offset: u64, max_phys: u64, regions_base: u64) -> Self {
        let rb = VirtAddr::new_truncate(regions_base);
        Self {
            hhdm_base: VirtAddr::new_truncate(hhdm_offset),
            hhdm_size: max_phys,
            regions_base: rb,
            heap: VirtRegion::new(rb + HEAP_OFFSET, HEAP_MAX_SIZE),
            stacks: VirtRegion::new(rb + STACKS_OFFSET, STACKS_MAX_SIZE),
            mmio: VirtRegion::new(rb + MMIO_OFFSET, MMIO_MAX_SIZE),
            percpu: VirtRegion::new(rb + PERCPU_OFFSET, PERCPU_MAX_SIZE),
            vdso: VirtRegion::new(rb + VDSO_OFFSET, VDSO_MAX_SIZE),
            kernel_image: VirtRegion::new(
                VirtAddr::new_truncate(KERNEL_IMAGE_BASE),
                KERNEL_IMAGE_MAX_SIZE,
            ),
        }
    }

    /// Identifies which kernel region contains `addr`.
    pub fn identify_region(&self, addr: VirtAddr) -> FaultRegion {
        if self.heap.contains(addr) {
            FaultRegion::Heap
        } else if self.stacks.contains(addr) {
            FaultRegion::Stacks
        } else if self.mmio.contains(addr) {
            FaultRegion::Mmio
        } else if self.percpu.contains(addr) {
            FaultRegion::PerCpu
        } else if self.kernel_image.contains(addr) {
            FaultRegion::KernelImage
        } else if addr.as_u64() >= self.hhdm_base.as_u64()
            && addr.as_u64() < self.hhdm_base.as_u64() + self.hhdm_size
        {
            FaultRegion::Hhdm
        } else {
            FaultRegion::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virt_region_contains_base() {
        let region = VirtRegion::new(VirtAddr::new(0x1000), 0x2000);
        assert!(region.contains(VirtAddr::new(0x1000)));
    }

    #[test]
    fn virt_region_contains_middle() {
        let region = VirtRegion::new(VirtAddr::new(0x1000), 0x2000);
        assert!(region.contains(VirtAddr::new(0x2000)));
    }

    #[test]
    fn virt_region_excludes_end() {
        let region = VirtRegion::new(VirtAddr::new(0x1000), 0x2000);
        assert!(!region.contains(VirtAddr::new(0x3000)));
    }

    #[test]
    fn virt_region_excludes_before() {
        let region = VirtRegion::new(VirtAddr::new(0x1000), 0x2000);
        assert!(!region.contains(VirtAddr::new(0x0FFF)));
    }

    #[test]
    fn virt_region_zero_size() {
        let region = VirtRegion::new(VirtAddr::new(0x1000), 0);
        assert!(!region.contains(VirtAddr::new(0x1000)));
    }

    #[test]
    fn memory_layout_default_base() {
        let layout = MemoryLayout::new(0xFFFF_8000_0000_0000, 0x1_0000_0000);
        assert_eq!(layout.regions_base.as_u64(), DEFAULT_REGIONS_BASE);
    }

    #[test]
    fn heap_at_base() {
        let layout = MemoryLayout::new(0xFFFF_8000_0000_0000, 0x1_0000_0000);
        // HEAP_OFFSET is 0, so heap base should equal regions_base.
        assert_eq!(layout.heap.base().as_u64(), layout.regions_base.as_u64());
    }

    #[test]
    fn regions_non_overlapping() {
        let layout = MemoryLayout::new(0xFFFF_8000_0000_0000, 0x1_0000_0000);
        assert!(
            layout.heap.end().as_u64() <= layout.stacks.base().as_u64(),
            "heap must end before stacks"
        );
        assert!(
            layout.stacks.end().as_u64() <= layout.mmio.base().as_u64(),
            "stacks must end before mmio"
        );
        assert!(
            layout.mmio.end().as_u64() <= layout.percpu.base().as_u64(),
            "mmio must end before percpu"
        );
    }

    #[test]
    fn custom_regions_base() {
        let custom_base = 0xFFFF_D000_0000_0000u64;
        let layout = MemoryLayout::with_regions_base(
            0xFFFF_8000_0000_0000,
            0x1_0000_0000,
            custom_base,
        );
        assert_eq!(
            layout.regions_base.as_u64(),
            VirtAddr::new_truncate(custom_base).as_u64()
        );
        assert_eq!(
            layout.heap.base().as_u64(),
            VirtAddr::new_truncate(custom_base).as_u64()
        );
    }

    #[test]
    fn identify_region_heap() {
        let layout = MemoryLayout::new(0xFFFF_8000_0000_0000, 0x1_0000_0000);
        let addr = layout.heap.base() + 0x1000;
        assert_eq!(layout.identify_region(addr), FaultRegion::Heap);
    }

    #[test]
    fn identify_region_stacks() {
        let layout = MemoryLayout::new(0xFFFF_8000_0000_0000, 0x1_0000_0000);
        let addr = layout.stacks.base() + 0x1000;
        assert_eq!(layout.identify_region(addr), FaultRegion::Stacks);
    }

    #[test]
    fn identify_region_mmio() {
        let layout = MemoryLayout::new(0xFFFF_8000_0000_0000, 0x1_0000_0000);
        let addr = layout.mmio.base() + 0x1000;
        assert_eq!(layout.identify_region(addr), FaultRegion::Mmio);
    }

    #[test]
    fn identify_region_percpu() {
        let layout = MemoryLayout::new(0xFFFF_8000_0000_0000, 0x1_0000_0000);
        let addr = layout.percpu.base() + 0x1000;
        assert_eq!(layout.identify_region(addr), FaultRegion::PerCpu);
    }

    #[test]
    fn identify_region_kernel_image() {
        let layout = MemoryLayout::new(0xFFFF_8000_0000_0000, 0x1_0000_0000);
        let addr = VirtAddr::new_truncate(KERNEL_IMAGE_BASE + 0x1000);
        assert_eq!(layout.identify_region(addr), FaultRegion::KernelImage);
    }

    #[test]
    fn identify_region_hhdm() {
        let hhdm_offset = 0xFFFF_8000_0000_0000u64;
        let layout = MemoryLayout::new(hhdm_offset, 0x1_0000_0000);
        let addr = VirtAddr::new_truncate(hhdm_offset + 0x1000);
        assert_eq!(layout.identify_region(addr), FaultRegion::Hhdm);
    }

    #[test]
    fn identify_region_unknown() {
        let layout = MemoryLayout::new(0xFFFF_8000_0000_0000, 0x1_0000_0000);
        let addr = VirtAddr::new(0x1000);
        assert_eq!(layout.identify_region(addr), FaultRegion::Unknown);
    }
}
