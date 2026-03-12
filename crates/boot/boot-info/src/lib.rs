//! Boot information shared between the UEFI stub and the kernel.
//!
//! This crate defines the `#[repr(C)]` structures that the UEFI boot stub
//! fills before jumping to `kernel_init`. Both the stub (UEFI target) and
//! the kernel (hadron target) depend on this crate.

#![no_std]
#![warn(missing_docs)]

/// Flags for boot-time page mapping requests.
#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
pub struct BootMapFlags(pub u64);

impl BootMapFlags {
    /// Map pages as writable.
    pub const WRITABLE: Self = Self(1 << 0);
    /// Map pages as non-executable.
    pub const NO_EXECUTE: Self = Self(1 << 1);
}

/// Boot services vtable. Valid only while the stub's page tables are in CR3.
///
/// This uses a manual `#[repr(C)]` vtable (not `dyn Trait`) because boot-info
/// is compiled for both UEFI (COFF) and kernel (ELF) targets, and Rust vtable
/// layout is not ABI-stable across targets.
#[repr(C)]
pub struct BootServices {
    /// Opaque context pointer passed to all callbacks.
    pub ctx: *mut (),
    /// Map `count` physical pages starting at `phys` into the HHDM.
    /// Returns the HHDM virtual address on success, or 0 on failure.
    pub map_pages:
        unsafe extern "C" fn(ctx: *mut (), phys: u64, count: u64, flags: BootMapFlags) -> u64,
}

// SAFETY: BootServices is only used during single-threaded early boot.
unsafe impl Send for BootServices {}
unsafe impl Sync for BootServices {}

/// Boot information passed from the UEFI stub to the kernel entry point.
///
/// The stub fills every field before calling `kernel_init`. The kernel must
/// not modify `BootInfo` after entry — it may reside in UEFI loader data
/// pages that the PMM will reclaim once the memory map is consumed.
#[repr(C)]
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

    /// Number of pages used by the boot stub from the page table pool.
    pub boot_pt_pool_pages: u64,

    /// Total number of pages in the boot page table pool.
    pub boot_pt_pool_total: u64,

    /// Boot services vtable. Valid until the kernel switches CR3. Null after.
    pub boot_services: *const BootServices,
}

// SAFETY: BootInfo is only shared during single-threaded early boot.
unsafe impl Send for BootInfo {}
unsafe impl Sync for BootInfo {}

impl core::fmt::Debug for BootInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BootInfo")
            .field("hhdm_offset", &self.hhdm_offset)
            .field("kaslr_slide", &self.kaslr_slide)
            .field("regions_base", &self.regions_base)
            .field("kernel_phys", &self.kernel_phys)
            .field("kernel_size", &self.kernel_size)
            .field("boot_services", &self.boot_services)
            .finish_non_exhaustive()
    }
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
