//! Binary loading and process creation.
//!
//! Parses a binary via the [`binfmt`](super::binfmt) registry, maps its
//! segments into a fresh user address space, sets up a user stack, and
//! returns a [`Process`] ready to run.

use hadron_core::addr::VirtAddr;
use hadron_core::arch::x86_64::registers::control::Cr3;
use hadron_core::mm::PAGE_SIZE;
use hadron_core::mm::address_space::AddressSpace;
use hadron_core::mm::mapper::MapFlags;
use hadron_core::mm::pmm::BitmapFrameAllocRef;
use hadron_core::paging::{Page, PhysFrame, Size4KiB};
use hadron_core::{kdebug, kinfo};

use super::Process;
use super::binfmt::{self, BinaryError, ExecSegment};

/// User stack top address. Placed just below the non-canonical hole.
/// Stack grows downward from here.
const USER_STACK_TOP: u64 = 0x7FFF_FFFF_F000;

/// User stack size: 64 KiB (16 pages) for MVP.
const USER_STACK_SIZE: u64 = 64 * 1024;

/// Frame deallocation callback for user address spaces.
///
/// Called by `AddressSpace::Drop` to free the PML4 frame.
fn dealloc_frame(frame: PhysFrame<Size4KiB>) {
    crate::mm::pmm::with_pmm(|pmm| {
        let mut dealloc = BitmapFrameAllocRef(pmm);
        // SAFETY: The frame was allocated by BitmapFrameAllocRef and is no
        // longer referenced by any page table (the address space is being dropped).
        unsafe {
            hadron_core::mm::FrameDeallocator::deallocate_frame(&mut dealloc, frame);
        }
    });
}

/// Loads a binary into a new user address space and returns the
/// process, entry point, and user stack top.
///
/// The caller is responsible for entering userspace via the executor.
pub fn create_process_from_binary(data: &[u8]) -> Result<(Process, u64, u64), BinaryError> {
    let image = binfmt::load_binary(data)?;
    let entry = image.entry_point;
    kinfo!("Loading process (entry={:#x})...", entry);

    // Get kernel state needed for address space creation.
    let kernel_cr3 = Cr3::read();
    let hhdm_offset = hadron_core::mm::hhdm::offset();

    #[cfg(target_arch = "x86_64")]
    type KernelMapper = hadron_core::arch::x86_64::paging::PageTableMapper;

    let mapper = KernelMapper::new(hhdm_offset);

    // Create address space and map segments + stack inside PMM lock scope.
    let process = crate::mm::pmm::with_pmm(|pmm| {
        let mut alloc = BitmapFrameAllocRef(pmm);

        // Create a new user address space (copies kernel upper half).
        // SAFETY: kernel_cr3 is the current (valid) PML4 read from CR3.
        // The mapper and allocator are correctly configured for the current
        // architecture. The allocator returns zeroed 4 KiB frames.
        let address_space = unsafe {
            AddressSpace::new_user(kernel_cr3, mapper, hhdm_offset, &mut alloc, dealloc_frame)
                .expect("failed to create user address space")
        };

        // Map binary segments.
        for seg in image.segments() {
            map_segment(&address_space, seg, hhdm_offset, &mut alloc);
        }

        // Map user stack.
        map_user_stack(&address_space, &mut alloc);

        // Wrap in Process (takes ownership of address space).
        Process::new(address_space)
    });

    Ok((process, entry, USER_STACK_TOP))
}

/// Maps a single loadable segment into the user address space.
#[expect(
    clippy::cast_possible_truncation,
    reason = "x86_64: u64 and usize are the same width"
)]
fn map_segment<
    M: hadron_core::mm::mapper::PageMapper<Size4KiB> + hadron_core::mm::mapper::PageTranslator,
>(
    address_space: &AddressSpace<M>,
    seg: &ExecSegment<'_>,
    hhdm_offset: u64,
    alloc: &mut BitmapFrameAllocRef<'_>,
) {
    let mut flags = MapFlags::USER;
    if seg.flags.writable {
        flags |= MapFlags::WRITABLE;
    }
    if seg.flags.executable {
        flags |= MapFlags::EXECUTABLE;
    }

    let page_mask = PAGE_SIZE as u64 - 1;
    let seg_start = seg.vaddr & !page_mask; // Page-align down
    let seg_end = (seg.vaddr + seg.memsz + page_mask) & !page_mask; // Page-align up
    let page_count = (seg_end - seg_start) / PAGE_SIZE as u64;

    kdebug!(
        "  Mapping segment: {:#x}..{:#x} ({} pages, flags={:?})",
        seg_start,
        seg_end,
        page_count,
        flags
    );

    for i in 0..page_count {
        let page_vaddr = seg_start + i * PAGE_SIZE as u64;
        let frame = alloc
            .0
            .allocate_frame()
            .expect("PMM: out of memory mapping segment");

        let page = Page::containing_address(VirtAddr::new(page_vaddr));

        // Map the page. Address space not yet in CR3, so ignore flush.
        address_space
            .map_user_page(page, frame, flags, alloc)
            .expect("failed to map segment page")
            .ignore();

        // Write the segment data into the frame via HHDM.
        let frame_phys = frame.start_address();
        let frame_ptr = (hhdm_offset + frame_phys.as_u64()) as *mut u8;

        // SAFETY: The frame was just allocated and mapped; zeroing via HHDM is safe.
        unsafe {
            core::ptr::write_bytes(frame_ptr, 0, PAGE_SIZE);
        }

        // Copy file data that overlaps this page.
        let page_offset_in_seg = page_vaddr.saturating_sub(seg.vaddr);
        let seg_data_start = page_offset_in_seg as usize;
        let seg_data_end = (seg_data_start + PAGE_SIZE).min(seg.data.len());

        if seg_data_start < seg.data.len() {
            let data = &seg.data[seg_data_start..seg_data_end];
            let dest_offset = if page_vaddr < seg.vaddr {
                (seg.vaddr - page_vaddr) as usize
            } else {
                0
            };
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    frame_ptr.add(dest_offset),
                    data.len(),
                );
            }
        }
    }
}

/// Maps a user stack (guard page + usable pages) and returns the stack top.
fn map_user_stack<
    M: hadron_core::mm::mapper::PageMapper<Size4KiB> + hadron_core::mm::mapper::PageTranslator,
>(
    address_space: &AddressSpace<M>,
    alloc: &mut BitmapFrameAllocRef<'_>,
) {
    let stack_bottom = USER_STACK_TOP - USER_STACK_SIZE;
    let page_count = USER_STACK_SIZE / PAGE_SIZE as u64;

    kdebug!(
        "  Mapping user stack: {:#x}..{:#x} ({} pages)",
        stack_bottom,
        USER_STACK_TOP,
        page_count
    );

    let flags = MapFlags::WRITABLE | MapFlags::USER;

    for i in 0..page_count {
        let page_vaddr = stack_bottom + i * PAGE_SIZE as u64;
        let frame = alloc
            .0
            .allocate_frame()
            .expect("PMM: out of memory mapping user stack");

        let page = Page::containing_address(VirtAddr::new(page_vaddr));

        // Address space not yet in CR3, so ignore flush.
        address_space
            .map_user_page(page, frame, flags, alloc)
            .expect("failed to map user stack page")
            .ignore();

        // SAFETY: The frame was just allocated and mapped; zeroing via HHDM is safe.
        let hhdm_offset = hadron_core::mm::hhdm::offset();
        let frame_ptr = (hhdm_offset + frame.start_address().as_u64()) as *mut u8;
        unsafe {
            core::ptr::write_bytes(frame_ptr, 0, PAGE_SIZE);
        }
    }
}
