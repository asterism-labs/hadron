//! Binary loading and process creation.
//!
//! Parses a binary via the [`binfmt`](super::binfmt) registry, maps its
//! segments into a fresh user address space, sets up a user stack, and
//! returns a [`Process`] ready to run.

use hadron_core::addr::VirtAddr;
use hadron_core::mm::PAGE_SIZE;
use hadron_core::mm::address_space::AddressSpace;
use hadron_core::mm::mapper::{MapFlags, PageMapper, PageTranslator};
use hadron_core::mm::pmm::BitmapFrameAllocRef;
use hadron_core::paging::{Page, PhysFrame, Size4KiB};
use hadron_core::{kdebug, kinfo};

extern crate alloc;

use alloc::sync::Arc;

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
///
/// # Errors
///
/// Returns [`BinaryError`] if format detection, parsing, or relocation fails.
///
/// # Panics
///
/// Panics if the user address space cannot be created or if physical memory
/// is exhausted while mapping segments or stack.
pub fn create_process_from_binary(
    data: &[u8],
    parent_pid: Option<u32>,
) -> Result<(Process, u64, u64), BinaryError> {
    #[cfg(target_arch = "x86_64")]
    type KernelMapper = hadron_core::arch::x86_64::paging::PageTableMapper;

    let image = binfmt::load_binary(data)?;
    let entry = image.entry_point;
    kinfo!("Loading process (entry={:#x})...", entry);

    // Use the saved kernel CR3 — not Cr3::read() — because this function may
    // be called from a syscall handler where CR3 is the calling process's
    // user page table, not the kernel's.
    let kernel_cr3 = super::kernel_cr3();
    let hhdm_offset = hadron_core::mm::hhdm::offset();
    let mapper = KernelMapper::new(hhdm_offset);

    // Create address space and map segments + stack inside PMM lock scope.
    let process = crate::mm::pmm::with_pmm(|pmm| -> Result<Process, BinaryError> {
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

        // Apply relocations for PIE binaries (ET_DYN).
        if image.needs_relocation {
            if let Some(elf_data) = image.elf_data {
                let elf = hadron_elf::ElfFile::parse(elf_data)
                    .expect("ELF already validated during load");
                binfmt::reloc::apply_dyn_relocations(
                    &address_space,
                    &elf,
                    image.base_addr,
                    hhdm_offset,
                )?;
            }
        }

        // Map user stack.
        map_user_stack(&address_space, &mut alloc);

        // Wrap in Process (takes ownership of address space).
        Ok(Process::new(address_space, parent_pid))
    })?;

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

/// Writes argv data onto the child's user stack via HHDM translation.
///
/// Stack layout (Rust-native `&str` = `(ptr, len)` in memory):
/// ```text
/// HIGH ADDRESS (USER_STACK_TOP = 0x7FFF_FFFF_F000)
///   ┌────────────────────────────────┐
///   │ arg string bytes (contiguous)  │  ← packed UTF-8, NOT NUL-terminated
///   ├────────────────────────────────┤
///   │ (ptr₀, len₀)  16 bytes        │  ← directly castable to &[&str]
///   │ (ptr₁, len₁)  16 bytes        │
///   │ ...                            │
///   ├────────────────────────────────┤
///   │ argc: usize                    │  ← RSP points here
///   └────────────────────────────────┘    (16-byte aligned)
/// ```
///
/// Returns the adjusted RSP value, or `BinaryError` if translation fails.
#[expect(
    clippy::cast_possible_truncation,
    reason = "x86_64: u64 and usize are the same width"
)]
fn write_argv_to_stack<M: PageMapper<Size4KiB> + PageTranslator>(
    address_space: &AddressSpace<M>,
    args: &[&str],
    hhdm_offset: u64,
) -> Result<u64, BinaryError> {
    if args.is_empty() {
        // Write just argc=0 at the top of the stack, 16-byte aligned.
        let argc_addr = USER_STACK_TOP - 16; // 16-byte aligned
        write_usize_to_user(address_space, argc_addr, 0, hhdm_offset)?;
        return Ok(argc_addr);
    }

    let mut cursor = USER_STACK_TOP;

    // 1. Write string bytes at the top of the stack (growing downward).
    //    Track the user-space virtual address of each string.
    let mut string_addrs = [(0u64, 0usize); 32]; // (vaddr, len), max 32 args
    for (i, arg) in args.iter().enumerate().rev() {
        cursor -= arg.len() as u64;
        let str_vaddr = cursor;
        // Write string bytes via HHDM.
        for (j, &byte) in arg.as_bytes().iter().enumerate() {
            let vaddr = str_vaddr + j as u64;
            let phys = address_space
                .translate(VirtAddr::new(vaddr))
                .ok_or(BinaryError::ParseError("argv string address not mapped"))?;
            let hhdm_ptr = (hhdm_offset + phys.as_u64()) as *mut u8;
            // SAFETY: The page was allocated by map_user_stack and zeroed.
            // Writing via HHDM before the address space is in CR3 is safe.
            unsafe {
                core::ptr::write(hhdm_ptr, byte);
            }
        }
        string_addrs[i] = (str_vaddr, arg.len());
    }

    // 2. Compute layout so that RSP is 16-byte aligned with:
    //    [RSP]     = argc  (8 bytes)
    //    [RSP + 8] = start of (ptr, len) pairs
    let pair_size = 2 * core::mem::size_of::<usize>() as u64; // 16 bytes on x86_64
    let total_below = core::mem::size_of::<usize>() as u64 + pair_size * args.len() as u64;
    let rsp = (cursor - total_below) & !0xF; // 16-byte aligned
    let argv_base = rsp + core::mem::size_of::<usize>() as u64;

    // 3. Write (ptr, len) pairs at argv_base.
    for (i, &(vaddr, len)) in string_addrs[..args.len()].iter().enumerate() {
        let pair_addr = argv_base + (i as u64) * pair_size;
        write_usize_to_user(address_space, pair_addr, vaddr as usize, hhdm_offset)?;
        write_usize_to_user(
            address_space,
            pair_addr + core::mem::size_of::<usize>() as u64,
            len,
            hhdm_offset,
        )?;
    }

    // 4. Write argc at RSP.
    write_usize_to_user(address_space, rsp, args.len(), hhdm_offset)?;

    Ok(rsp)
}

/// Writes a `usize` value to a virtual address in the child's address space via HHDM.
fn write_usize_to_user<M: PageMapper<Size4KiB> + PageTranslator>(
    address_space: &AddressSpace<M>,
    vaddr: u64,
    value: usize,
    hhdm_offset: u64,
) -> Result<(), BinaryError> {
    let phys = address_space
        .translate(VirtAddr::new(vaddr))
        .ok_or(BinaryError::ParseError("argv address not mapped in child"))?;
    let hhdm_ptr = (hhdm_offset + phys.as_u64()) as *mut usize;
    // SAFETY: The page was allocated by map_user_stack and the address space
    // is not yet loaded into CR3. Writing via HHDM is safe.
    unsafe {
        core::ptr::write_unaligned(hhdm_ptr, value);
    }
    Ok(())
}

/// Writes argv for the init process: `["/init"]`.
///
/// This is a separate entry point for `spawn_init` which doesn't go through
/// the full `spawn_process` flow.
///
/// # Errors
///
/// Returns [`BinaryError`] if address translation fails.
pub fn write_argv_to_init_stack<M: PageMapper<Size4KiB> + PageTranslator>(
    address_space: &AddressSpace<M>,
    hhdm_offset: u64,
) -> Result<u64, BinaryError> {
    write_argv_to_stack(address_space, &["/init"], hhdm_offset)
}

/// Spawns a new process from an ELF binary at the given VFS path.
///
/// Reads the binary from the VFS, creates a process with inherited fd 0/1/2
/// from the parent, writes argv onto the child stack, registers it in the
/// global process table, and spawns its async task on the executor.
///
/// Returns the child process `Arc` on success.
///
/// # Errors
///
/// Returns [`BinaryError`] if the path cannot be resolved, the file cannot
/// be read, or the binary cannot be loaded.
pub fn spawn_process(
    path: &str,
    parent_pid: u32,
    args: &[&str],
) -> Result<Arc<Process>, BinaryError> {
    use crate::fs::file::OpenFlags;
    use crate::fs::{poll_immediate, vfs};

    let inode = vfs::with_vfs(|vfs| vfs.resolve(path))
        .map_err(|_| BinaryError::ParseError("path not found"))?;

    let file_size = inode.size();
    let mut buf = alloc::vec![0u8; file_size];
    let bytes_read = poll_immediate(inode.read(0, &mut buf))
        .map_err(|_| BinaryError::ParseError("failed to read binary"))?;
    assert_eq!(bytes_read, file_size, "short read of binary");

    let (process, entry, _stack_top) = create_process_from_binary(&buf, Some(parent_pid))?;

    // Write argv onto the child's user stack.
    let hhdm_offset = hadron_core::mm::hhdm::offset();
    let stack_top = write_argv_to_stack(process.address_space(), args, hhdm_offset)?;

    // Inherit fd 0/1/2 from parent, or fall back to /dev/console.
    {
        let parent = super::lookup_process(parent_pid).expect("spawn_process: parent not found");
        let parent_fds = parent.fd_table.lock();
        let console = vfs::with_vfs(|vfs| {
            vfs.resolve("/dev/console")
                .expect("spawn_process: /dev/console not found")
        });
        let mut fd_table = process.fd_table.lock();
        for fd_num in 0..=2usize {
            if let Some(parent_fd) = parent_fds.get(fd_num) {
                fd_table.insert_at(fd_num, parent_fd.inode.clone(), parent_fd.flags);
            } else {
                let flags = if fd_num == 0 {
                    OpenFlags::READ
                } else {
                    OpenFlags::WRITE
                };
                fd_table.insert_at(fd_num, console.clone(), flags);
            }
        }
    }

    let process = Arc::new(process);
    super::register_process(&process);

    kinfo!(
        "Process {}: spawning child of {} (entry={:#x}, stack={:#x})",
        process.pid,
        parent_pid,
        entry,
        stack_top
    );

    crate::sched::spawn(super::process_task(process.clone(), entry, stack_top));

    Ok(process)
}
