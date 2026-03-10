//! Boot information shared between the UEFI stub and the kernel.
//!
//! This crate defines the `#[repr(C)]` structures that the UEFI boot stub
//! fills before jumping to `kernel_init`. Both the stub (UEFI target) and
//! the kernel (hadron target) depend on this crate.

#![no_std]
#![warn(missing_docs)]

/// Boot information passed from the UEFI stub to the kernel entry point.
///
/// The stub fills every field before calling `kernel_init`. The kernel must
/// not modify `BootInfo` after entry — it may reside in UEFI loader data
/// pages that the PMM will reclaim once the memory map is consumed.
#[repr(C)]
#[derive(Debug)]
pub struct BootInfo {
    /// Pointer to the UEFI memory descriptor array (physical address).
    pub memory_map_ptr: u64,
    /// Number of entries in the memory map.
    pub memory_map_len: usize,
    /// Size of each memory descriptor in bytes (firmware-reported stride).
    pub memory_descriptor_size: usize,

    /// Physical address of the ACPI RSDP (Root System Description Pointer).
    pub rsdp_phys: u64,

    /// Linear framebuffer descriptor.
    pub framebuffer: FramebufferInfo,

    /// Physical address of the initial ramdisk.
    pub initrd_phys: u64,
    /// Size of the initial ramdisk in bytes.
    pub initrd_len: usize,

    /// HHDM (Higher Half Direct Map) virtual base address.
    pub hhdm_offset: u64,

    /// KASLR slide applied to kernel virtual address (0 = disabled).
    pub kaslr_slide: u64,

    /// Base address for kernel virtual regions (heap, stacks, MMIO).
    /// Kernel uses this instead of `DEFAULT_REGIONS_BASE` when non-zero.
    pub regions_base: u64,

    /// Physical address where the kernel image was loaded.
    pub kernel_phys: u64,

    /// Size of the kernel image in bytes (page-aligned).
    pub kernel_size: u64,

    /// Physical base of the boot page table pool (for reclamation).
    pub boot_pt_pool_phys: u64,

    /// Number of pages in the boot page table pool.
    pub boot_pt_pool_pages: u64,
}

/// Linear framebuffer descriptor provided by the UEFI GOP.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    /// Physical base address of the framebuffer.
    pub base_phys: u64,
    /// Size of the framebuffer in bytes.
    pub size: usize,
    /// Horizontal resolution in pixels.
    pub width: u32,
    /// Vertical resolution in pixels.
    pub height: u32,
    /// Pixels per scan line (may exceed `width` due to padding).
    pub stride: u32,
    /// Pixel format.
    pub format: PixelFormat,
}

/// Pixel format of the linear framebuffer.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Blue-Green-Red byte order (UEFI `BlueGreenRedReserved8BitPerColor`).
    Bgr = 0,
    /// Red-Green-Blue byte order (UEFI `RedGreenBlueReserved8BitPerColor`).
    Rgb = 1,
}
