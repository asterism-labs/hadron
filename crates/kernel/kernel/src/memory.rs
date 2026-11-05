//! Boot-time memory setup: the kernel's own page tables, the physical
//! memory map and the frame allocator.
//!
//! Limine starts the kernel on its own page tables, with its direct map
//! (the HHDM) at an offset it chooses. [`init`] replaces them in two
//! stages:
//!
//! 1. On Limine's tables, a bump allocator takes frames from the largest
//!    usable region. They hold new page tables that map:
//!    - the kernel's direct map at [`DIRECT_MAP`], read-write and
//!      non-executable;
//!    - the kernel image at its link address, each part with its own
//!      permissions, so no page is both writable and executable;
//!    - a boot stack with an unmapped guard page below it;
//!    - Limine's direct map again at Limine's offset, so the boot
//!      responses and the old stack stay readable.
//!
//!    Then CR3 switches to the new tables and RSP to the boot stack.
//! 2. On the new tables, the kernel copies Limine's memory map into a
//!    [`MemoryMap`] and builds the bitmap frame allocator. The frames the
//!    bump allocator handed out stay marked in use.
//!
//! Everything built in stage 1 is kept as physical addresses, so it stays
//! valid after the direct map moves. The Limine alias stays mapped until
//! the kernel reclaims bootloader memory, which it does not do yet.
//!
//! After [`init`], three statics hold the results, since interrupt
//! handlers and the global allocator need them and have no caller to pass
//! them in:
//!
//! - [`map`]: the memory map, written once and read without a lock;
//! - [`frames`]: the frame allocator, an [`IrqSpinLock`] at
//!   [`levels::Frames`];
//! - [`kernel_tables`]: the kernel's page tables, an [`IrqSpinLock`] at
//!   [`levels::KernelTables`], which comes before the frame allocator.

use core::mem::size_of;

use arch_x86_64::paging::{PAGE_SIZE, PageSize, PageTableFlags};
use arch_x86_64::{PhysAddr, VirtAddr};
use limine::memmap::{MemMapEntry, MemMapEntryType};
use limine::paging::PagingMode;
use limine::{ExecutableAddressRequest, HhdmRequest, MemMapRequest, PagingModeRequest};
use mm::{
    BitmapFrameAllocator, BumpAllocator, DirectMap, FrameAllocator, MapError, Mapper, MemoryMap,
    PhysFrame, Region, RegionKind, Translation,
};

use crate::arch::x86_64::cpu;
use crate::sync::{Ctx, IrqSpinLock, IrqSpinLockGuard, LockBefore, WriteOnce, levels};

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

/// The kernel's direct map, PML4 slot 273. It differs from Limine's
/// default offset, slot 256, so a pointer into Limine's map that outlives
/// the alias faults instead of reading the right memory by luck.
pub(crate) const DIRECT_MAP: DirectMap = DirectMap::new(0xffff_8880_0000_0000);

/// Room reserved for the direct map: 64 TiB of physical memory.
const DIRECT_MAP_SIZE: u64 = 64 << 40;

/// Start of the kernel heap, PML4 slot 401.
pub(crate) const HEAP: VirtAddr = VirtAddr::new(0xffff_c880_0000_0000);

/// Size of the heap. It does not grow yet.
pub(crate) const HEAP_SIZE: u64 = 4 << 20;

/// Start of the kernel stack region, PML4 slot 402. The first page is the
/// boot stack's guard page.
const STACKS: VirtAddr = VirtAddr::new(0xffff_c900_0000_0000);

/// Size of a kernel thread's stack.
pub(crate) const KERNEL_STACK_SIZE: u64 = 16 * 1024;

/// Size of the boot stack.
const BOOT_STACK_SIZE: u64 = 64 * 1024;

/// The lowest byte of the boot stack. The page below it is unmapped.
const BOOT_STACK_BOTTOM: VirtAddr = VirtAddr::new(STACKS.as_u64() + PAGE_SIZE);

/// One past the highest byte of the boot stack.
const BOOT_STACK_TOP: VirtAddr = VirtAddr::new(BOOT_STACK_BOTTOM.as_u64() + BOOT_STACK_SIZE);

/// Where kernel thread stacks start, right after the boot stack. Each
/// starts with a guard page.
pub(crate) const THREAD_STACKS: VirtAddr = BOOT_STACK_TOP;

const _: () = assert!(DIRECT_MAP.offset() + DIRECT_MAP_SIZE <= HEAP.as_u64());
const _: () = assert!(HEAP.as_u64() + HEAP_SIZE <= STACKS.as_u64());

static MAP: WriteOnce<MemoryMap<'static>> = WriteOnce::new();
type Frames = BitmapFrameAllocator<'static>;

static FRAMES: WriteOnce<IrqSpinLock<levels::Frames, Frames>> = WriteOnce::new();
static KERNEL_TABLES: WriteOnce<IrqSpinLock<levels::KernelTables, Mapper>> = WriteOnce::new();

const INIT_RAN: &str = "memory::init ran";

/// What each range of physical memory holds.
///
/// # Panics
///
/// Panics before [`init`].
pub(crate) fn map() -> &'static MemoryMap<'static> {
    MAP.get().expect(INIT_RAN)
}

/// Locks the frame allocator.
///
/// # Panics
///
/// Panics before [`init`].
pub(crate) fn frames<'a, P: LockBefore<levels::Frames>>(
    ctx: &'a mut Ctx<'_, P>,
) -> IrqSpinLockGuard<'a, levels::Frames, Frames> {
    FRAMES.get().expect(INIT_RAN).lock(ctx)
}

/// Locks the kernel's page tables, the ones in CR3, and returns a token
/// for taking the frame allocator next.
///
/// # Panics
///
/// Panics before [`init`].
pub(crate) fn kernel_tables<'a, P: LockBefore<levels::KernelTables>>(
    ctx: &'a mut Ctx<'_, P>,
) -> (
    IrqSpinLockGuard<'a, levels::KernelTables, Mapper>,
    Ctx<'a, levels::KernelTables>,
) {
    KERNEL_TABLES.get().expect(INIT_RAN).lock_and(ctx)
}

/// Maps `len` bytes at `virt` to new frames, page by page, read-write and
/// non-executable. The frames are not zeroed.
///
/// # Errors
///
/// Returns an error if frames run out or part of the range is mapped. The
/// pages mapped before the error stay mapped.
///
/// # Panics
///
/// Panics before [`init`].
pub(crate) fn map_new_pages<P: LockBefore<levels::KernelTables>>(
    ctx: &mut Ctx<'_, P>,
    virt: VirtAddr,
    len: u64,
) -> Result<(), MapError> {
    let (mut tables, mut ctx) = kernel_tables(ctx);
    let mut frames = frames(&mut ctx);
    let mut page = virt;
    while page < virt + len {
        let frame = frames.allocate().ok_or(MapError::OutOfFrames)?;
        tables.map(
            page,
            frame.start(),
            PageSize::Size4K,
            PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
            &mut *frames,
        )?;
        page = page + PAGE_SIZE;
    }
    Ok(())
}

/// What stage 1 passes to stage 2 across the stack switch.
struct Handoff {
    bump: BumpAllocator,
    root: PhysFrame,
    next: fn() -> !,
}

/// Switches to the kernel's page tables and boot stack, builds the memory
/// map and frame allocator, and calls `next`. Limine's stack is abandoned.
///
/// # Panics
///
/// Panics if called twice, if Limine did not answer the memory requests,
/// or if the memory map is unusable.
pub(crate) fn init(next: fn() -> !) -> ! {
    let hhdm = HHDM.response().expect("Limine answers the HHDM request");
    let limine = DirectMap::new(hhdm.hhdm_base);
    let memmap = MEMMAP
        .response()
        .expect("Limine answers the memory map request");
    let executable = EXECUTABLE_ADDRESS
        .response()
        .expect("Limine answers the executable address request");
    if let Some(paging) = PAGING_MODE.response() {
        assert_eq!(paging.paging_mode, PagingMode::Paging4Level);
    }

    let largest = memmap
        .entries()
        .filter(|entry| entry.type_ == MemMapEntryType::Usable)
        .max_by_key(|entry| entry.length)
        .expect("the memory map has a usable region");
    let start = PhysAddr::new(largest.base);
    let mut bump = BumpAllocator::new(start, start + largest.length);

    // SAFETY: Limine's direct map covers usable memory, and the bump
    // allocator hands out usable frames no one else owns.
    let mut mapper = unsafe { Mapper::empty(limine, &mut bump) }.expect(BOOT_MEMORY);
    map_direct(&mut mapper, DIRECT_MAP, memmap.entries(), &mut bump);
    if limine != DIRECT_MAP {
        map_direct(&mut mapper, limine, memmap.entries(), &mut bump);
    }
    let image = KernelImage {
        virt_base: executable.virt_base,
        phys_base: executable.phys_base,
    };
    image.map(&mut mapper, &mut bump);
    map_boot_stack(&mut mapper, &mut bump);

    let handoff = Handoff {
        bump,
        root: mapper.root(),
        next,
    };
    // SAFETY: the new tables map the kernel image at its link address and
    // Limine's direct map at its offset, so this code, the current stack
    // and `handoff` stay reachable. The boot stack is mapped, writable and
    // unused, and `handoff` lives on the abandoned stack, which stays
    // mapped through the alias.
    unsafe {
        cpu::switch_page_tables(handoff.root);
        cpu::switch_stack(BOOT_STACK_TOP.as_u64(), stage2, &raw const handoff as u64)
    }
}

/// Stage 2, on the kernel's page tables and boot stack.
extern "C" fn stage2(handoff: u64) -> ! {
    // SAFETY: `init` passes a pointer to a `Handoff` it never uses again.
    let Handoff {
        mut bump,
        root,
        next,
    } = unsafe { (handoff as *const Handoff).read() };
    // SAFETY: CR3 holds `root`, and every frame it uses is mapped at
    // `DIRECT_MAP`. Nothing else changes the tables.
    let kernel = unsafe { Mapper::new(root, DIRECT_MAP) };
    let memmap = MEMMAP
        .response()
        .expect("Limine answered the memory map request");

    #[allow(clippy::cast_possible_truncation)] // fits on a 64-bit target
    let count = memmap.entry_count as usize;
    let storage = allocate_slice(&mut bump, count, Region::EMPTY);
    let map = MemoryMap::build(storage, memmap.entries().map(region))
        .expect("Limine's memory map has no overlaps");

    let frame_count = map.allocatable_end().as_u64() / PAGE_SIZE;
    let words = allocate_slice(&mut bump, BitmapFrameAllocator::words_for(frame_count), 0);
    let mut frames = BitmapFrameAllocator::new(words, frame_count);
    for usable in map.of_kind(RegionKind::Usable) {
        frames.mark_free(usable.start, usable.end);
    }
    let (used_start, used_end) = bump.used();
    frames.mark_used(used_start, used_end);

    MAP.init(map);
    FRAMES.init(IrqSpinLock::new(frames));
    KERNEL_TABLES.init(IrqSpinLock::new(kernel));
    next()
}

const BOOT_MEMORY: &str = "the largest usable region holds the boot page tables";

/// Maps every range Limine's direct map covers at `direct`, read-write and
/// non-executable. Ranges are rounded out to pages and merged when they
/// touch, since two entries can share a page.
fn map_direct<'a>(
    mapper: &mut Mapper,
    direct: DirectMap,
    entries: impl Iterator<Item = &'a MemMapEntry>,
    frames: &mut impl FrameAllocator,
) {
    let mut map = |start: PhysAddr, end: PhysAddr| {
        assert!(
            end.as_u64() <= DIRECT_MAP_SIZE,
            "physical memory past the direct map"
        );
        // 1 GiB pages need CPUID.80000001H:EDX.Page1GB, which QEMU's
        // default CPU lacks, so the largest is 2 MiB.
        mapper
            .map_range(
                direct.phys_to_virt(start),
                start,
                end - start,
                PageSize::Size2M,
                PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
                frames,
            )
            .expect(BOOT_MEMORY);
    };
    let mut pending: Option<(PhysAddr, PhysAddr)> = None;
    for entry in entries.filter(|entry| region(entry).kind.is_direct_mapped()) {
        let start = PhysAddr::new(entry.base).align_down(PAGE_SIZE);
        let end = PhysAddr::new(entry.base + entry.length).align_up(PAGE_SIZE);
        pending = match pending {
            Some((s, e)) if start <= e => Some((s, e.max(end))),
            Some((s, e)) => {
                map(s, e);
                Some((start, end))
            }
            None => Some((start, end)),
        };
    }
    if let Some((s, e)) = pending {
        map(s, e);
    }
}

/// Maps the boot stack below [`BOOT_STACK_TOP`], leaving the page under it
/// unmapped.
fn map_boot_stack(mapper: &mut Mapper, bump: &mut BumpAllocator) {
    let stack = bump
        .allocate_frames(BOOT_STACK_SIZE / PAGE_SIZE)
        .expect(BOOT_MEMORY);
    mapper
        .map_range(
            BOOT_STACK_BOTTOM,
            stack.start(),
            BOOT_STACK_SIZE,
            PageSize::Size4K,
            PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
            bump,
        )
        .expect(BOOT_MEMORY);
}

/// Takes frames for `len` `T`s from `bump`, fills them with `value`, and
/// returns them. The frames stay in use for good.
fn allocate_slice<T: Copy>(bump: &mut BumpAllocator, len: usize, value: T) -> &'static mut [T] {
    let bytes = (len * size_of::<T>()) as u64;
    let frame = bump
        .allocate_frames(bytes.div_ceil(PAGE_SIZE).max(1))
        .expect(BOOT_MEMORY);
    // SAFETY: the frames are fresh from the bump allocator, page aligned,
    // mapped at `DIRECT_MAP` for good, and never handed out again: the
    // frame allocator marks everything the bump allocator used.
    unsafe { DIRECT_MAP.init_slice(frame.start(), len, value) }
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

unsafe extern "C" {
    static __requests_start: u8;
    static __requests_end: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
}

/// The parts of the kernel image `kernel.ld` lays out, with the
/// permissions each gets.
fn image_parts() -> [(VirtAddr, VirtAddr, PageTableFlags); 4] {
    let addr = |symbol: *const u8| VirtAddr::from_ptr(symbol);
    let nx = PageTableFlags::NO_EXECUTE;
    // Limine writes the responses before the kernel starts, so the
    // requests are read-only from here on.
    [
        (
            addr(&raw const __requests_start),
            addr(&raw const __requests_end),
            nx,
        ),
        (
            addr(&raw const __text_start),
            addr(&raw const __text_end),
            PageTableFlags::EMPTY,
        ),
        (
            addr(&raw const __rodata_start),
            addr(&raw const __rodata_end),
            nx,
        ),
        (
            addr(&raw const __data_start),
            addr(&raw const __data_end),
            PageTableFlags::WRITABLE | nx,
        ),
    ]
}

/// Where Limine loaded the kernel image. It is physically contiguous.
struct KernelImage {
    virt_base: u64,
    phys_base: u64,
}

impl KernelImage {
    /// Maps each part of the image at its link address.
    fn map(&self, mapper: &mut Mapper, frames: &mut impl FrameAllocator) {
        for (start, end, flags) in image_parts() {
            let start = start.align_down(PAGE_SIZE);
            let len = (end.as_u64() - start.as_u64()).next_multiple_of(PAGE_SIZE);
            let phys = PhysAddr::new(start.as_u64() - self.virt_base + self.phys_base);
            mapper
                .map_range(start, phys, len, PageSize::Size4K, flags, frames)
                .expect(BOOT_MEMORY);
        }
    }
}

/// What the active page tables map at `virt`.
pub(crate) fn translate(virt: VirtAddr) -> Option<Translation> {
    // SAFETY: CR3 holds the kernel's tables, which `DIRECT_MAP` covers, and
    // `translate` only reads them.
    let mapper = unsafe { Mapper::new(cpu::cr3(), DIRECT_MAP) };
    mapper.translate(virt)
}

ktest::ktest! {
    fn kernel_image_is_writable_or_executable_never_both() {
        let w = PageTableFlags::WRITABLE;
        let nx = PageTableFlags::NO_EXECUTE;
        let [requests, text, rodata, data] = image_parts();
        for (start, end, expected) in [requests, text, rodata, data] {
            let mut page = start.align_down(PAGE_SIZE);
            while page < end {
                let flags = translate(page).expect("the image is mapped").flags;
                assert_eq!(flags.contains(w), expected.contains(w), "{page:?}");
                assert_eq!(flags.contains(nx), expected.contains(nx), "{page:?}");
                page = page + PAGE_SIZE;
            }
        }
    }
}

ktest::ktest! {
    fn direct_map_reaches_the_page_tables() {
        let root = cpu::cr3().start();
        let virt = DIRECT_MAP.phys_to_virt(root);
        let t = translate(virt).expect("the direct map covers the root table");
        assert_eq!(t.phys, root);
        assert!(t.flags.contains(PageTableFlags::NO_EXECUTE | PageTableFlags::WRITABLE));
    }
}

ktest::ktest! {
    fn boot_stack_has_a_guard_page() {
        assert_eq!(translate(STACKS), None);
        assert!(translate(BOOT_STACK_BOTTOM).is_some());
    }
}

ktest::ktest! {
    fn null_page_is_unmapped() {
        assert_eq!(translate(VirtAddr::ZERO), None);
    }
}
