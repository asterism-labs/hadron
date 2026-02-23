//! Binary loading and process creation.
//!
//! Parses a binary via the [`binfmt`](super::binfmt) registry, maps its
//! segments into a fresh user address space, sets up a user stack, and
//! returns a [`Process`] ready to run.

use crate::addr::VirtAddr;
use crate::id::Pid;
use crate::mm::PAGE_SIZE;
use crate::mm::address_space::AddressSpace;
use crate::mm::mapper::{MapFlags, PageMapper, PageTranslator};
use crate::mm::pmm::BitmapFrameAllocRef;
use crate::paging::{Page, PhysFrame, Size4KiB};
use crate::{kdebug, kinfo};

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;

use super::Process;

/// Options for `spawn_process` controlling fd inheritance and child CWD.
pub struct SpawnOptions<'a> {
    /// Explicit fd remapping: `(child_fd, parent_fd)` pairs.
    /// If `None`, the default behavior applies (inherit fds 0/1/2).
    pub fd_map: Option<&'a [(u32, u32)]>,
    /// Working directory for the child process.
    /// If `None`, the child inherits the parent's CWD.
    pub cwd: Option<String>,
}
use super::binfmt::{self, BinaryError, ExecSegment};

/// User stack top address. Placed just below the non-canonical hole.
/// Stack grows downward from here.
const USER_STACK_TOP: u64 = 0x7FFF_FFFF_F000;

/// User stack size: 64 KiB (16 pages) for MVP.
const USER_STACK_SIZE: u64 = 64 * 1024;

/// Syscall number for `task_sigreturn`, used in the trampoline stub.
const SYS_TASK_SIGRETURN_NR: u64 = {
    // task group base 0x00 + offset 0x07
    0x07
};

/// Frame deallocation callback for user address spaces.
///
/// Called by `AddressSpace::Drop` to free the PML4 frame.
fn dealloc_frame(frame: PhysFrame<Size4KiB>) {
    crate::mm::pmm::with(|pmm| {
        let mut dealloc = BitmapFrameAllocRef(pmm);
        // SAFETY: The frame was allocated by BitmapFrameAllocRef and is no
        // longer referenced by any page table (the address space is being dropped).
        unsafe {
            crate::mm::FrameDeallocator::deallocate_frame(&mut dealloc, frame);
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
    parent_pid: Option<Pid>,
) -> Result<(Process, u64, u64), BinaryError> {
    #[cfg(target_arch = "x86_64")]
    type KernelMapper = crate::arch::x86_64::paging::PageTableMapper;

    let image = binfmt::load_binary(data)?;
    let entry = image.entry_point;
    kinfo!("Loading process (entry={:#x})...", entry);

    // Use the saved kernel CR3 — not Cr3::read() — because this function may
    // be called from a syscall handler where CR3 is the calling process's
    // user page table, not the kernel's.
    let kernel_cr3 = super::TrapContext::kernel_cr3();
    let hhdm_offset = crate::mm::hhdm::offset();
    let mapper = KernelMapper::new(hhdm_offset);

    // Create address space and map segments + stack inside PMM lock scope.
    let process = crate::mm::pmm::with(|pmm| -> Result<Process, BinaryError> {
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

        // Map signal return trampoline page.
        map_sigreturn_trampoline(&address_space, hhdm_offset, &mut alloc);

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
fn map_segment<M: crate::mm::mapper::PageMapper<Size4KiB> + crate::mm::mapper::PageTranslator>(
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
    M: crate::mm::mapper::PageMapper<Size4KiB> + crate::mm::mapper::PageTranslator,
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
        let hhdm_offset = crate::mm::hhdm::offset();
        let frame_ptr = (hhdm_offset + frame.start_address().as_u64()) as *mut u8;
        unsafe {
            core::ptr::write_bytes(frame_ptr, 0, PAGE_SIZE);
        }
    }
}

/// Maps a single read-only executable page at [`super::SIGRETURN_TRAMPOLINE_ADDR`]
/// containing a tiny code stub that calls `task_sigreturn()`.
///
/// The stub is: `mov rax, SYS_TASK_SIGRETURN; syscall`
/// (x86_64 machine code: `48 c7 c0 07 00 00 00 0f 05`).
///
/// Every user process gets this page so signal handlers can return
/// through it without needing a per-binary trampoline symbol.
fn map_sigreturn_trampoline<
    M: crate::mm::mapper::PageMapper<Size4KiB> + crate::mm::mapper::PageTranslator,
>(
    address_space: &AddressSpace<M>,
    hhdm_offset: u64,
    alloc: &mut BitmapFrameAllocRef<'_>,
) {
    let trampoline_addr = super::SIGRETURN_TRAMPOLINE_ADDR;
    let page = Page::containing_address(VirtAddr::new(trampoline_addr));
    let frame = alloc
        .0
        .allocate_frame()
        .expect("PMM: out of memory mapping sigreturn trampoline");

    // Read + execute, no write — user can execute but not modify.
    let flags = MapFlags::USER | MapFlags::EXECUTABLE;

    address_space
        .map_user_page(page, frame, flags, alloc)
        .expect("failed to map sigreturn trampoline page")
        .ignore();

    // Write the trampoline stub into the frame via HHDM.
    let frame_ptr = (hhdm_offset + frame.start_address().as_u64()) as *mut u8;
    // SAFETY: The frame was just allocated and is not yet accessible from userspace
    // (address space not in CR3). Writing via HHDM is safe.
    unsafe {
        // Zero the page first.
        core::ptr::write_bytes(frame_ptr, 0, PAGE_SIZE);

        // Write: mov rax, SYS_TASK_SIGRETURN (0x07)
        //   48 c7 c0 07 00 00 00    mov rax, 0x7
        // Write: syscall
        //   0f 05                   syscall
        let stub: [u8; 9] = [
            0x48,
            0xc7,
            0xc0,
            SYS_TASK_SIGRETURN_NR as u8,
            0x00,
            0x00,
            0x00,
            0x0f,
            0x05,
        ];
        core::ptr::copy_nonoverlapping(stub.as_ptr(), frame_ptr, stub.len());
    }

    kdebug!("  Mapped sigreturn trampoline at {:#x}", trampoline_addr);
}

/// Writes argv and envp data onto the child's user stack via HHDM translation.
///
/// Stack layout (Rust-native `&str` = `(ptr, len)` in memory):
/// ```text
/// HIGH ADDRESS (USER_STACK_TOP = 0x7FFF_FFFF_F000)
///   ┌────────────────────────────────┐
///   │ env string bytes (contiguous)  │
///   │ arg string bytes (contiguous)  │  ← packed UTF-8, NOT NUL-terminated
///   ├────────────────────────────────┤
///   │ envp (ptr, len) pairs          │
///   │ argv (ptr, len) pairs          │  ← directly castable to &[&str]
///   ├────────────────────────────────┤
///   │ envc: usize                    │
///   │ argc: usize                    │  ← RSP points here
///   └────────────────────────────────┘    (16-byte aligned)
/// ```
///
/// Returns the adjusted RSP value, or `BinaryError` if translation fails.
#[expect(
    clippy::cast_possible_truncation,
    reason = "x86_64: u64 and usize are the same width"
)]
fn write_startup_data<M: PageMapper<Size4KiB> + PageTranslator>(
    address_space: &AddressSpace<M>,
    args: &[&str],
    envs: &[&str],
    hhdm_offset: u64,
) -> Result<u64, BinaryError> {
    if args.is_empty() && envs.is_empty() {
        // Write argc=0, envc=0 at the top of the stack, 16-byte aligned.
        let base = USER_STACK_TOP - 16; // 16-byte aligned
        write_usize_to_user(address_space, base, 0, hhdm_offset)?; // argc
        write_usize_to_user(
            address_space,
            base + core::mem::size_of::<usize>() as u64,
            0,
            hhdm_offset,
        )?; // envc
        return Ok(base);
    }

    let mut cursor = USER_STACK_TOP;

    // Maximum combined entries for strings.
    const MAX_STRINGS: usize = 96; // 32 args + 64 envs
    let mut string_addrs = [(0u64, 0usize); MAX_STRINGS];

    // 1. Write env string bytes first (higher addresses).
    for (i, env) in envs.iter().enumerate().rev() {
        let idx = args.len() + i;
        cursor -= env.len() as u64;
        let str_vaddr = cursor;
        write_string_bytes(address_space, str_vaddr, env, hhdm_offset)?;
        string_addrs[idx] = (str_vaddr, env.len());
    }

    // 2. Write arg string bytes.
    for (i, arg) in args.iter().enumerate().rev() {
        cursor -= arg.len() as u64;
        let str_vaddr = cursor;
        write_string_bytes(address_space, str_vaddr, arg, hhdm_offset)?;
        string_addrs[i] = (str_vaddr, arg.len());
    }

    // 3. Compute layout:
    //    [RSP]      = argc (8 bytes)
    //    [RSP + 8]  = envc (8 bytes)
    //    [RSP + 16] = argv (ptr, len) pairs
    //    [RSP + 16 + argc * 16] = envp (ptr, len) pairs
    let pair_size = 2 * core::mem::size_of::<usize>() as u64; // 16 bytes on x86_64
    let header_size = 2 * core::mem::size_of::<usize>() as u64; // argc + envc
    let total_below = header_size + pair_size * args.len() as u64 + pair_size * envs.len() as u64;
    let rsp = (cursor - total_below) & !0xF; // 16-byte aligned

    // 4. Write argc and envc.
    write_usize_to_user(address_space, rsp, args.len(), hhdm_offset)?;
    write_usize_to_user(
        address_space,
        rsp + core::mem::size_of::<usize>() as u64,
        envs.len(),
        hhdm_offset,
    )?;

    // 5. Write argv (ptr, len) pairs.
    let argv_base = rsp + header_size;
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

    // 6. Write envp (ptr, len) pairs.
    let envp_base = argv_base + pair_size * args.len() as u64;
    for (i, &(vaddr, len)) in string_addrs[args.len()..args.len() + envs.len()]
        .iter()
        .enumerate()
    {
        let pair_addr = envp_base + (i as u64) * pair_size;
        write_usize_to_user(address_space, pair_addr, vaddr as usize, hhdm_offset)?;
        write_usize_to_user(
            address_space,
            pair_addr + core::mem::size_of::<usize>() as u64,
            len,
            hhdm_offset,
        )?;
    }

    Ok(rsp)
}

/// Write a UTF-8 string's bytes to the user address space via HHDM.
fn write_string_bytes<M: PageMapper<Size4KiB> + PageTranslator>(
    address_space: &AddressSpace<M>,
    vaddr: u64,
    s: &str,
    hhdm_offset: u64,
) -> Result<(), BinaryError> {
    for (j, &byte) in s.as_bytes().iter().enumerate() {
        let addr = vaddr + j as u64;
        let phys = address_space
            .translate(VirtAddr::new(addr))
            .ok_or(BinaryError::ParseError("string address not mapped"))?;
        let hhdm_ptr = (hhdm_offset + phys.as_u64()) as *mut u8;
        // SAFETY: The page was allocated by map_user_stack and zeroed.
        // Writing via HHDM before the address space is in CR3 is safe.
        unsafe {
            core::ptr::write(hhdm_ptr, byte);
        }
    }
    Ok(())
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

/// Writes startup data for the init process: argv=`["/bin/init"]`, no envp.
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
    write_startup_data(address_space, &["/bin/init"], &[], hhdm_offset)
}

/// Spawns a new process from an ELF binary at the given VFS path.
///
/// Reads the binary from the VFS, creates a process with inherited fd 0/1/2
/// from the parent, writes argv and envp onto the child stack, registers it
/// in the global process table, and spawns its async task on the executor.
///
/// Returns the child process `Arc` on success.
///
/// # Errors
///
/// Returns [`BinaryError`] if the path cannot be resolved, the file cannot
/// be read, or the binary cannot be loaded.
pub fn spawn_process(
    path: &str,
    parent_pid: Pid,
    args: &[&str],
    envs: &[&str],
    opts: Option<SpawnOptions<'_>>,
) -> Result<Arc<Process>, BinaryError> {
    use crate::fs::file::OpenFlags;
    use crate::fs::{poll_immediate, vfs};
    use crate::id::Fd;

    let inode = vfs::with_vfs(|vfs| vfs.resolve(path))
        .map_err(|_| BinaryError::ParseError("path not found"))?;

    let file_size = inode.size();
    let mut buf = alloc::vec![0u8; file_size];
    let bytes_read = poll_immediate(inode.read(0, &mut buf))
        .map_err(|_| BinaryError::ParseError("failed to read binary"))?;
    assert_eq!(bytes_read, file_size, "short read of binary");

    let (process, entry, _stack_top) = create_process_from_binary(&buf, Some(parent_pid))?;

    // Write argv and envp onto the child's user stack.
    let hhdm_offset = crate::mm::hhdm::offset();
    let stack_top = write_startup_data(&*process.address_space(), args, envs, hhdm_offset)?;

    let fd_map = opts.as_ref().and_then(|o| o.fd_map);
    let child_cwd = opts.as_ref().and_then(|o| o.cwd.clone());

    // Inherit file descriptors into the child process.
    // Resolve /dev/console BEFORE locking any fd_table to maintain the
    // VFS -> fd_table ordering (same as spawn_init).
    {
        let console = vfs::with_vfs(|vfs| {
            vfs.resolve("/dev/console")
                .expect("spawn_process: /dev/console not found")
        });
        let parent =
            super::ProcessTable::lookup(parent_pid).expect("spawn_process: parent not found");
        let parent_fds = parent.fd_table.lock();
        let mut fd_table = process.fd_table.lock();

        if let Some(fd_map) = fd_map {
            // Explicit fd remapping: only the listed fds are inherited.
            for &(child_fd, parent_fd) in fd_map {
                let child_fd = Fd::new(child_fd);
                let parent_fd = Fd::new(parent_fd);
                if let Some(src) = parent_fds.get(parent_fd) {
                    fd_table.insert_at(child_fd, src.inode.clone(), src.flags);
                }
            }
        } else {
            // Default: inherit fds 0/1/2 from parent.
            for &fd in &[Fd::STDIN, Fd::STDOUT, Fd::STDERR] {
                if let Some(parent_fd) = parent_fds.get(fd) {
                    fd_table.insert_at(fd, parent_fd.inode.clone(), parent_fd.flags);
                } else {
                    let flags = if fd == Fd::STDIN {
                        OpenFlags::READ
                    } else {
                        OpenFlags::WRITE
                    };
                    fd_table.insert_at(fd, console.clone(), flags);
                }
            }
        }
    }

    // Set child CWD: use explicit value, or inherit from parent.
    if let Some(cwd) = child_cwd {
        *process.cwd.lock() = cwd;
    } else {
        let parent =
            super::ProcessTable::lookup(parent_pid).expect("spawn_process: parent not found");
        *process.cwd.lock() = parent.cwd.lock().clone();
    }

    let process = Arc::new(process);
    super::ProcessTable::register(&process);

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

// ── Execve ─────────────────────────────────────────────────────────

/// Public constants for use by the Exec trap handler.
pub const USER_MMAP_BASE: u64 = 0x0000_4000_0000_0000;
/// Maximum mmap region size.
pub const USER_MMAP_MAX_SIZE: u64 = 256 * 1024 * 1024 * 1024 * 1024;

/// Handle `task_execve`: load a new binary and replace the process's address space.
///
/// Called from `process_task` under user CR3 (to read SpawnInfo from user memory).
/// Returns `(entry_point, stack_top)` on success, or negated errno on failure.
///
/// On success, the process's address space has been replaced (old one dropped).
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning negated errno as isize"
)]
pub fn handle_execve(
    process: &Process,
    info_ptr: usize,
    info_len: usize,
) -> Result<(u64, u64), isize> {
    use crate::syscall::{EINVAL, ENOENT};
    use hadron_syscall::SpawnInfo;

    if info_len < core::mem::size_of::<SpawnInfo>() {
        return Err(EINVAL);
    }

    // SAFETY: Caller switched to user CR3. info_ptr is in user memory.
    let info = unsafe { core::ptr::read(info_ptr as *const SpawnInfo) };

    // Read path from user memory.
    let path_slice =
        unsafe { core::slice::from_raw_parts(info.path_ptr as *const u8, info.path_len) };
    let path = core::str::from_utf8(path_slice).map_err(|_| EINVAL)?;

    // Read argv and envp from user memory.
    let args = read_user_string_array(info.argv_ptr, info.argv_count);
    let envs = read_user_string_array(info.envp_ptr, info.envp_count);

    // Switch back to kernel CR3 for VFS operations.
    unsafe {
        crate::arch::x86_64::registers::control::Cr3::write(super::TrapContext::kernel_cr3());
    }

    // Resolve and read the binary from VFS.
    let inode = crate::fs::vfs::with_vfs(|vfs| vfs.resolve(path)).map_err(|_| ENOENT)?;
    let file_size = inode.size();
    let mut buf = alloc::vec![0u8; file_size];
    crate::fs::poll_immediate(inode.read(0, &mut buf)).map_err(|_| ENOENT)?;
    let binary_data = buf;

    // Load the binary and create a new address space.
    let (new_space, entry, _stack_top) = match create_address_space_from_binary(&binary_data) {
        Ok(result) => result,
        Err(_e) => return Err(EINVAL),
    };

    // Write argv/envp onto the new stack.
    let hhdm_offset = crate::mm::hhdm::offset();
    let args_refs: alloc::vec::Vec<&str> = args.iter().map(alloc::string::String::as_str).collect();
    let envs_refs: alloc::vec::Vec<&str> = envs.iter().map(alloc::string::String::as_str).collect();
    let stack_top = match write_startup_data(&new_space, &args_refs, &envs_refs, hhdm_offset) {
        Ok(st) => st,
        Err(_e) => return Err(EINVAL),
    };

    // Replace the process's address space (drops the old one).
    let _old_space = process.replace_address_space(new_space);

    // Switch to the new user CR3 for subsequent operations.
    unsafe {
        crate::arch::x86_64::registers::control::Cr3::write(process.user_cr3());
    }

    kinfo!(
        "Process {}: execve to {} (entry={:#x}, stack={:#x})",
        process.pid,
        path,
        entry,
        stack_top
    );

    Ok((entry, stack_top))
}

/// Read an array of C strings from user memory.
fn read_user_string_array(ptr: usize, count: usize) -> alloc::vec::Vec<String> {
    if ptr == 0 || count == 0 {
        return alloc::vec::Vec::new();
    }
    let mut result = alloc::vec::Vec::with_capacity(count);
    // SAFETY: ptr points to an array of (ptr, len) pairs in user memory,
    // and caller has switched to user CR3.
    let pairs = unsafe { core::slice::from_raw_parts(ptr as *const [usize; 2], count) };
    for pair in pairs {
        let s_ptr = pair[0];
        let s_len = pair[1];
        if s_ptr == 0 || s_len == 0 {
            result.push(String::new());
            continue;
        }
        let bytes = unsafe { core::slice::from_raw_parts(s_ptr as *const u8, s_len) };
        result.push(String::from_utf8_lossy(bytes).into_owned());
    }
    result
}

/// Create a new address space from binary data without creating a Process.
///
/// Returns `(AddressSpace, entry_point, stack_top)`.
fn create_address_space_from_binary(
    data: &[u8],
) -> Result<
    (
        AddressSpace<crate::arch::x86_64::paging::PageTableMapper>,
        u64,
        u64,
    ),
    BinaryError,
> {
    #[cfg(target_arch = "x86_64")]
    type KernelMapper = crate::arch::x86_64::paging::PageTableMapper;

    let image = binfmt::load_binary(data)?;
    let entry = image.entry_point;

    let kernel_cr3 = super::TrapContext::kernel_cr3();
    let hhdm_offset = crate::mm::hhdm::offset();
    let mapper = KernelMapper::new(hhdm_offset);

    let address_space = crate::mm::pmm::with(|pmm| -> Result<_, BinaryError> {
        let mut alloc = BitmapFrameAllocRef(pmm);

        // SAFETY: Same as create_process_from_binary.
        let address_space = unsafe {
            AddressSpace::new_user(kernel_cr3, mapper, hhdm_offset, &mut alloc, dealloc_frame)
                .expect("failed to create user address space")
        };

        for seg in image.segments() {
            map_segment(&address_space, seg, hhdm_offset, &mut alloc);
        }

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

        map_user_stack(&address_space, &mut alloc);
        map_sigreturn_trampoline(&address_space, hhdm_offset, &mut alloc);

        Ok(address_space)
    })?;

    Ok((address_space, entry, USER_STACK_TOP))
}
