//! Boot information and UEFI memory map conversion.
//!
//! Re-exports [`BootInfo`] and provides conversion from UEFI memory
//! descriptors to [`PhysMemoryRegion`] slices for PMM initialization.

pub use hadron_boot_info::BootInfo;

use hadron_core::addr::{PhysAddr, VirtAddr};
use hadron_mm::PhysMemoryRegion;

/// Maximum number of physical memory regions we can convert.
const MAX_REGIONS: usize = 256;

/// UEFI memory type constants (from UEFI spec, matches `EfiMemoryType`).
mod efi_memory_type {
    pub const LOADER_CODE: u32 = 1;
    pub const LOADER_DATA: u32 = 2;
    pub const BOOT_SERVICES_CODE: u32 = 3;
    pub const BOOT_SERVICES_DATA: u32 = 4;
    pub const CONVENTIONAL_MEMORY: u32 = 7;
    pub const ACPI_RECLAIM_MEMORY: u32 = 9;
}

/// Result of converting the UEFI memory map.
pub struct ConvertedMemoryMap {
    /// Physical memory regions.
    pub regions: [PhysMemoryRegion; MAX_REGIONS],
    /// Number of valid entries in `regions`.
    pub count: usize,
    /// Highest physical address across all regions (for VMM layout).
    pub max_phys: u64,
}

/// Converts the UEFI memory descriptor array into a `PhysMemoryRegion` slice.
///
/// Reads descriptors via raw pointer arithmetic using the known `#[repr(C)]`
/// layout: `memory_type` at offset 0 (u32), `physical_start` at offset 8
/// (u64), `number_of_pages` at offset 24 (u64). The `descriptor_size` stride
/// handles any firmware-specific padding.
///
/// # Safety
///
/// - `bi` must point to a valid `BootInfo` with a correct memory map.
/// - `hhdm_offset` must be the initialized HHDM base so the physical
///   `memory_map_ptr` can be accessed.
pub unsafe fn convert_uefi_memory_map(bi: &BootInfo, hhdm_offset: VirtAddr) -> ConvertedMemoryMap {
    let mut result = ConvertedMemoryMap {
        regions: [PhysMemoryRegion {
            start: PhysAddr::new(0),
            size: 0,
            usable: false,
        }; MAX_REGIONS],
        count: 0,
        max_phys: 0,
    };

    let map_virt = hhdm_offset.as_u64() + bi.memory_map_ptr;
    let stride = bi.memory_descriptor_size;

    for i in 0..bi.memory_map_len {
        if result.count >= MAX_REGIONS {
            break;
        }

        // SAFETY: The UEFI stub set up the memory map and the HHDM maps it.
        // We read fields at known offsets within the EfiMemoryDescriptor layout.
        let desc_base = unsafe { (map_virt as *const u8).add(i * stride) };
        let memory_type = unsafe { desc_base.cast::<u32>().read_unaligned() };
        let physical_start = unsafe { desc_base.add(8).cast::<u64>().read_unaligned() };
        let number_of_pages = unsafe { desc_base.add(24).cast::<u64>().read_unaligned() };

        let size = number_of_pages * 4096;
        let end = physical_start + size;

        let usable = matches!(
            memory_type,
            efi_memory_type::CONVENTIONAL_MEMORY
                | efi_memory_type::LOADER_CODE
                | efi_memory_type::LOADER_DATA
                | efi_memory_type::BOOT_SERVICES_CODE
                | efi_memory_type::BOOT_SERVICES_DATA
        );

        // Include ACPI reclaim as non-usable (reserved until tables are parsed).
        let include = usable || memory_type == efi_memory_type::ACPI_RECLAIM_MEMORY;

        if include {
            result.regions[result.count] = PhysMemoryRegion {
                start: PhysAddr::new(physical_start),
                size,
                usable,
            };
            result.count += 1;

            if end > result.max_phys {
                result.max_phys = end;
            }
        }
    }

    result
}
