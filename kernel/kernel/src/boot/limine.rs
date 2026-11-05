//! The Limine adapter: the requests, the entry point Limine jumps to, and
//! the [`BootInfo`] built from Limine's responses.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use arch_x86_64::{PhysAddr, VirtAddr};
use limine::memmap::{MemMapEntry, MemMapEntryType};
use limine::paging::PagingMode;
use limine::{
    BaseRevision, BootloaderInfoRequest, ExecutableAddressRequest, HhdmRequest, MemMapRequest,
    PagingModeRequest, RequestsEndMarker, RequestsStartMarker,
};
use mm::{DirectMap, Region, RegionKind};

use super::BootInfo;
use crate::console::kprintln;

#[used]
#[unsafe(link_section = ".limine_requests_start")]
static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static BOOTLOADER_INFO: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static HHDM: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static MEMMAP: MemMapRequest = MemMapRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static EXECUTABLE_ADDRESS: ExecutableAddressRequest = ExecutableAddressRequest::new();

/// The kernel uses 4-level paging only. Limine picks 4-level paging when
/// the request is missing, but the request makes the choice explicit.
#[used]
#[unsafe(link_section = ".limine_requests")]
static PAGING_MODE: PagingModeRequest = PagingModeRequest::new(
    PagingMode::Paging4Level,
    PagingMode::Paging4Level,
    PagingMode::Paging4Level,
);

#[used]
#[unsafe(link_section = ".limine_requests_end")]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

/// The most memory map entries the kernel takes. Limine merges adjacent
/// entries of the same kind, so maps stay far below this.
const MAX_REGIONS: usize = 256;

/// Storage for the converted memory map, since nothing can allocate yet.
struct RegionBuffer {
    taken: AtomicBool,
    regions: UnsafeCell<[Region; MAX_REGIONS]>,
}

// SAFETY: `take_regions` hands out the array once, so no two threads reach
// it.
unsafe impl Sync for RegionBuffer {}

static REGIONS: RegionBuffer = RegionBuffer {
    taken: AtomicBool::new(false),
    regions: UnsafeCell::new([Region::EMPTY; MAX_REGIONS]),
};

/// The region array in [`REGIONS`].
///
/// # Panics
///
/// Panics if called twice.
fn take_regions() -> &'static mut [Region; MAX_REGIONS] {
    let taken = REGIONS.taken.swap(true, Ordering::Relaxed);
    assert!(!taken, "the region buffer is taken once");
    // SAFETY: the swap lets only the first caller through, so this is the
    // only reference to the array.
    unsafe { &mut *REGIONS.regions.get() }
}

/// Boots the kernel from Limine. The binary's entry point calls this.
///
/// # Safety
///
/// The caller is the entry point Limine jumps to, and nothing ran before
/// it. Limine enters the kernel once, on the boot CPU alone, in 64-bit
/// mode with interrupts masked, the kernel mapped at its link address,
/// and a stack of at least 64 KiB.
pub unsafe fn enter() -> ! {
    // SAFETY: the caller is Limine's entry point, as above. `PAGING_MODE`
    // asks for 4-level paging. `kernel_main` checks the CPU state.
    // `boot_info` explains why its `BootInfo` is sound.
    unsafe { super::kernel_main(boot_info) }
}

/// Reads Limine's responses into a [`BootInfo`]. The responses live in
/// bootloader-reclaimable memory, so nothing reads them after this.
///
/// # Panics
///
/// Panics if Limine does not support the base revision, does not answer a
/// request the kernel needs, or reports more than [`MAX_REGIONS`] memory
/// map entries.
fn boot_info() -> BootInfo {
    assert!(
        BASE_REVISION.is_supported(),
        "Limine supports base revision {}",
        BASE_REVISION.revision_slot()
    );
    if let Some(revision) = BASE_REVISION.loaded_revision() {
        kprintln!("hadron: Limine base revision {revision}");
    }
    let hhdm = HHDM.response().expect("Limine answers the HHDM request");
    let memmap = MEMMAP
        .response()
        .expect("Limine answers the memory map request");
    let executable = EXECUTABLE_ADDRESS
        .response()
        .expect("Limine answers the executable address request");

    #[allow(clippy::cast_possible_truncation)] // fits on a 64-bit target
    let count = memmap.entry_count as usize;
    assert!(
        count <= MAX_REGIONS,
        "the memory map has at most {MAX_REGIONS} entries, not {count}"
    );
    let regions = take_regions();
    for (slot, entry) in regions.iter_mut().zip(memmap.entries()) {
        *slot = region(entry);
    }
    let regions: &'static [Region; MAX_REGIONS] = regions;

    let (name, version) = BOOTLOADER_INFO
        .response()
        .map_or(("an unknown bootloader", ""), |info| {
            (info.name(), info.version())
        });

    // SAFETY: `take_regions` succeeds once, so this is the only
    // `BootInfo`. The Limine protocol guarantees the rest: the memory map
    // is accurate, the HHDM maps every entry except reserved and bad
    // memory, the executable address response gives where the image was
    // loaded, and the strings live in bootloader-reclaimable memory.
    unsafe {
        BootInfo::new(
            &regions[..count],
            DirectMap::new(hhdm.hhdm_base),
            PhysAddr::new(executable.phys_base),
            VirtAddr::new(executable.virt_base),
            name,
            version,
        )
    }
}

/// A Limine memory map entry as a [`Region`].
fn region(entry: &MemMapEntry) -> Region {
    let kind = match entry.type_ {
        MemMapEntryType::Usable => RegionKind::Usable,
        MemMapEntryType::Reserved => RegionKind::Reserved,
        MemMapEntryType::AcpiReclaimable => RegionKind::AcpiReclaimable,
        MemMapEntryType::AcpiNvs => RegionKind::AcpiNvs,
        MemMapEntryType::BadMemory => RegionKind::BadMemory,
        MemMapEntryType::BootloaderReclaimable => RegionKind::BootloaderReclaimable,
        MemMapEntryType::KernelAndModules => RegionKind::ExecutableAndModules,
        MemMapEntryType::Framebuffer => RegionKind::Framebuffer,
        MemMapEntryType::ReservedMapped => RegionKind::ReservedMapped,
    };
    let start = PhysAddr::new(entry.base);
    Region {
        start,
        end: start + entry.length,
        kind,
    }
}
