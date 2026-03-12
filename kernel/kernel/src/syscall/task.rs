//! Task (process lifecycle) syscall handlers.
//!
//! Implements `task_exit`, `task_spawn`, `task_wait`, and `task_info`.
//! Other task syscalls (kill, sigaction, setpgid, getpgid) are stubbed.

extern crate alloc;

use alloc::string::ToString;
use alloc::sync::Arc;

use hadron_core::addr::VirtAddr;
use hadron_core::paging::{Page, PhysFrame, Size4KiB};
use hadron_mm::address_space::AddressSpace;
use hadron_mm::mapper::MapFlags;
use hadron_objects::object::KernelObject;
use hadron_objects::process::Process;
use hadron_objects::thread::Thread;
use hadron_objects::vmar::Vmar;
use hadron_syscall::*;

use super::validate::{UserPtrMut, UserSlice};
use crate::arch::x86_64::paging::PageTableMapper;
use crate::process::{self, BlockingOp};

/// Page size constant.
const PAGE_SIZE: u64 = 4096;
/// Page offset mask.
const PAGE_MASK: u64 = PAGE_SIZE - 1;
/// ELF segment flag: executable.
const PF_X: u32 = 1;
/// ELF segment flag: writable.
const PF_W: u32 = 2;
/// Default user stack top for spawned processes.
const CHILD_STACK_TOP: u64 = 0x0000_7FFF_FFFF_0000;
/// Number of pages for a child's user stack (64 KiB).
const CHILD_STACK_PAGES: u64 = 16;
/// User VMAR base and size.
const USER_BASE: u64 = 0x0000_0010_0000_0000;
const USER_SIZE: u64 = 0x0000_7FEF_0000_0000;

/// `SYS_TASK_EXIT(code)` — terminate the current task.
///
/// Stores a [`BlockingOp::Exit`] and longjmps back to the process task
/// via `restore_kernel_context`.
pub fn sys_task_exit(code: usize) -> isize {
    process::set_blocking_op(BlockingOp::Exit(code));

    // Read the saved kernel RSP from percpu.kernel_rsp (gs:[8]).
    let saved_rsp: u64;
    // SAFETY: GS is kernel (set by syscall_entry swapgs). gs:[8] was
    // set by enter_userspace_save before entering ring 3.
    unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };

    // SAFETY: saved_rsp was written by enter_userspace_save. The kernel
    // stack at that point is intact (syscall handler used space below it).
    unsafe {
        crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
    }
}

/// Maximum number of fd_map entries allowed per spawn.
const MAX_FD_MAP_ENTRIES: usize = 64;

/// `SYS_TASK_SPAWN(info_ptr, info_len)` — spawn a new process.
///
/// `info_ptr` points to a [`SpawnInfo`] struct. `info_len` must equal
/// `size_of::<SpawnInfo>()`. The binary is looked up in the embedded
/// initrd table. Handle inheritance is controlled by the `fd_map` array
/// in `SpawnInfo`.
///
/// Returns the child's PID (koid) on success, or a negative error code.
#[expect(
    clippy::cast_possible_truncation,
    reason = "koid raw value fits in isize on x86_64"
)]
pub fn sys_task_spawn(info_ptr: usize, info_len: usize) -> isize {
    use hadron_syscall::types::{FdMapEntry, SpawnInfo};

    // Validate and read SpawnInfo from user memory.
    if info_len != core::mem::size_of::<SpawnInfo>() {
        return -EINVAL;
    }
    let info_uptr = match super::validate::UserPtr::<SpawnInfo>::new(info_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    // SAFETY: Pointer was range-validated and size matches.
    let info = unsafe { info_uptr.read() };

    // Read the path string.
    let path_slice = match UserSlice::new(info.path_ptr, info.path_len) {
        Ok(s) => s,
        Err(e) => return e,
    };
    // SAFETY: User buffer was range-validated.
    let path_bytes = unsafe { path_slice.read_to_vec() };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(p) => p,
        Err(_) => return -EINVAL,
    };

    // Read fd_map entries (if any).
    let fd_map = if info.fd_map_count > 0 {
        if info.fd_map_count > MAX_FD_MAP_ENTRIES {
            return -EINVAL;
        }
        let fd_map_size = info.fd_map_count * core::mem::size_of::<FdMapEntry>();
        let fd_slice = match UserSlice::new(info.fd_map_ptr, fd_map_size) {
            Ok(s) => s,
            Err(e) => return e,
        };
        // SAFETY: User buffer was range-validated.
        let fd_bytes = unsafe { fd_slice.read_to_vec() };
        // SAFETY: FdMapEntry is repr(C) with no padding concerns for u32 fields.
        let entries: alloc::vec::Vec<FdMapEntry> = fd_bytes
            .chunks_exact(core::mem::size_of::<FdMapEntry>())
            .map(|chunk| {
                let child_fd = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let parent_fd = u32::from_ne_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                FdMapEntry {
                    child_fd,
                    parent_fd,
                }
            })
            .collect();
        entries
    } else {
        alloc::vec::Vec::new()
    };

    crate::kinfo!("syscall", "task_spawn: \"{}\"", path);

    // Look up the binary in the embedded initrd.
    let elf_bytes = match crate::userboot::lookup_initrd_binary(path) {
        Some(bytes) => bytes,
        None => {
            crate::kwarn!("syscall", "task_spawn: binary not found: {}", path);
            return -ENOENT;
        }
    };

    // Parse the ELF.
    let elf = match hadron_elf::ElfFile::parse(elf_bytes) {
        Ok(e) => e,
        Err(_) => return -EINVAL,
    };
    let entry = elf.entry_point();

    // Create a new address space for the child.
    let kernel_cr3 = crate::arch::x86_64::registers::control::Cr3::read();
    let hhdm_offset = hadron_mm::hhdm::offset();
    let mapper = PageTableMapper::new(hhdm_offset);

    let address_space = hadron_mm::pmm::with(|pmm| {
        let mut alloc = hadron_mm::pmm::BitmapFrameAllocRef(pmm);
        // SAFETY: kernel_cr3 is a valid PML4; alloc returns zeroed frames.
        unsafe {
            AddressSpace::new_user(kernel_cr3, mapper, hhdm_offset, &mut alloc, dealloc_frame)
        }
    });

    let address_space = match address_space {
        Ok(a) => a,
        Err(_) => return -ENOMEM,
    };

    // Map ELF segments into the child's address space.
    for seg in elf.load_segments() {
        let seg_vaddr = seg.vaddr;
        let seg_memsz = seg.memsz;
        let page_start = seg_vaddr & !PAGE_MASK;
        let page_end = (seg_vaddr + seg_memsz + PAGE_MASK) & !PAGE_MASK;

        let mut flags = MapFlags::empty();
        if seg.flags & PF_W != 0 {
            flags |= MapFlags::WRITABLE;
        }
        if seg.flags & PF_X != 0 {
            flags |= MapFlags::EXECUTABLE;
        }

        let mut page_addr = page_start;
        while page_addr < page_end {
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_addr));
            let frame = hadron_mm::pmm::with(|pmm| {
                pmm.allocate_frame()
                    .expect("PMM: out of frames for child ELF")
            });

            // Zero + copy via HHDM.
            let hhdm_ptr = hadron_mm::hhdm::phys_to_virt(frame.start_address());
            // SAFETY: Frame was just allocated and is HHDM-mapped.
            let page_slice = unsafe {
                core::slice::from_raw_parts_mut(hhdm_ptr.as_u64() as *mut u8, PAGE_SIZE as usize)
            };
            page_slice.fill(0);

            let copy_start = page_addr.max(seg_vaddr);
            let data_end = seg_vaddr + seg.data.len() as u64;
            let copy_end = (page_addr + PAGE_SIZE).min(data_end);
            if copy_start < copy_end {
                let dst_offset = (copy_start - page_addr) as usize;
                let src_offset = (copy_start - seg_vaddr) as usize;
                let len = (copy_end - copy_start) as usize;
                page_slice[dst_offset..dst_offset + len]
                    .copy_from_slice(&seg.data[src_offset..src_offset + len]);
            }

            hadron_mm::pmm::with(|pmm| {
                let mut alloc = hadron_mm::pmm::BitmapFrameAllocRef(pmm);
                address_space
                    .map_user_page(page, frame, flags, &mut alloc)
                    .expect("failed to map child ELF page")
                    .flush();
            });

            page_addr += PAGE_SIZE;
        }
    }

    // Map child's user stack.
    let stack_bottom = CHILD_STACK_TOP - CHILD_STACK_PAGES * PAGE_SIZE;
    let mut top_page_hhdm = VirtAddr::zero();
    for i in 0..CHILD_STACK_PAGES {
        let page_addr = stack_bottom + i * PAGE_SIZE;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_addr));
        let frame = hadron_mm::pmm::with(|pmm| {
            pmm.allocate_frame()
                .expect("PMM: out of frames for child stack")
        });

        let hhdm_ptr = hadron_mm::hhdm::phys_to_virt(frame.start_address());
        // SAFETY: Frame was just allocated and is HHDM-mapped.
        let page_slice = unsafe {
            core::slice::from_raw_parts_mut(hhdm_ptr.as_u64() as *mut u8, PAGE_SIZE as usize)
        };
        page_slice.fill(0);

        if i == CHILD_STACK_PAGES - 1 {
            top_page_hhdm = hhdm_ptr;
        }

        hadron_mm::pmm::with(|pmm| {
            let mut alloc = hadron_mm::pmm::BitmapFrameAllocRef(pmm);
            let flags = MapFlags::WRITABLE;
            address_space
                .map_user_page(page, frame, flags, &mut alloc)
                .expect("failed to map child stack page")
                .flush();
        });
    }

    // Write a minimal C-style argc/argv/envp layout at the top of the stack.
    // Layout (growing downward from CHILD_STACK_TOP):
    //   [RSP + 0]  = argc (0)
    //   [RSP + 8]  = argv NULL terminator
    //   [RSP + 16] = envp NULL terminator
    //   [RSP + 24] = padding (16-byte alignment)
    let user_rsp = CHILD_STACK_TOP - 32;
    let offset_in_page = (PAGE_SIZE - 32) as usize;
    // SAFETY: top_page_hhdm points to the HHDM mapping of the topmost stack frame,
    // which was just allocated and zeroed. We write 4 qwords at the end of the page.
    unsafe {
        let base = (top_page_hhdm.as_u64() as *mut u64).add(offset_in_page / 8);
        base.write(0); // argc = 0
        base.add(1).write(0); // argv[0] = NULL
        base.add(2).write(0); // envp[0] = NULL
        base.add(3).write(0); // padding
    }

    // Create Process and Thread objects.
    let root_vmar = Vmar::new_root(USER_BASE, USER_SIZE);
    let child_process = Process::new(path.to_string(), root_vmar);
    let child_thread = Thread::new("main".to_string(), &child_process);
    child_process.add_thread(Arc::clone(&child_thread));

    let child_pid = child_process.koid().raw();

    // Process fd_map: duplicate parent handles into child's handle table.
    if !fd_map.is_empty() {
        let result = super::with_handle_table(|parent_table| {
            child_process.with_handle_table(|child_table| {
                for entry in &fd_map {
                    let parent_hv = hadron_objects::handle::HandleValue::from_raw(entry.parent_fd);
                    let child_hv = hadron_objects::handle::HandleValue::from_raw(entry.child_fd);

                    let parent_entry = match parent_table.get(parent_hv) {
                        Ok(e) => e,
                        Err(_) => return -EBADF,
                    };

                    let new_entry = hadron_objects::handle::HandleEntry::new(
                        Arc::clone(parent_entry.object()),
                        parent_entry.rights(),
                    );

                    if child_table.insert_at(child_hv, new_entry).is_err() {
                        return -EMFILE;
                    }
                }
                0
            })
        });
        if result != 0 {
            return result;
        }
    }

    // Inherit parent namespace and CWD.
    process::with_current_process(|parent| {
        child_process.inherit_namespace(parent);
        child_process.set_cwd(parent.cwd());
    });

    // Register in global process table.
    process::register_process(&child_process);

    crate::kinfo!(
        "syscall",
        "spawned child process {} (\"{}\")",
        child_pid,
        path
    );

    // Spawn the child's process_task on the executor.
    hadron_sched::spawn(process::process_task(
        child_process,
        child_thread,
        Some(address_space),
        entry,
        user_rsp,
    ));

    child_pid as isize
}

/// `SYS_TASK_WAIT(pid, status_ptr, flags)` — wait for a child to exit.
///
/// Blocking syscall: stores [`BlockingOp::TaskWait`] and longjmps back
/// to the process task, which awaits the child's TERMINATED signal.
pub fn sys_task_wait(pid: usize, status_ptr: usize, _flags: usize) -> isize {
    // Validate status_ptr if non-zero.
    if status_ptr != 0 {
        if let Err(e) = UserPtrMut::<usize>::new(status_ptr) {
            return e;
        }
    }

    // Verify the child exists.
    if process::lookup_process(pid as u64).is_none() {
        return -ENOENT;
    }

    process::set_blocking_op(BlockingOp::TaskWait {
        pid: pid as u64,
        status_ptr,
    });

    // Longjmp back to the process task.
    let saved_rsp: u64;
    // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
    unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
    // SAFETY: saved_rsp is valid.
    unsafe {
        crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
    }
}

/// `SYS_TASK_INFO()` — return current process koid (PID).
pub fn sys_task_info() -> isize {
    process::with_current_process(|proc| proc.koid().raw() as isize).unwrap_or(-ENOSYS)
}

/// `SYS_TASK_KILL` — stub (not yet implemented).
pub fn sys_task_kill(_pid: usize, _signum: usize) -> isize {
    -ENOSYS
}

/// `SYS_TASK_SIGACTION` — stub (not yet implemented).
pub fn sys_task_sigaction(
    _signum: usize,
    _handler: usize,
    _flags: usize,
    _old_handler_ptr: usize,
) -> isize {
    -ENOSYS
}

/// `SYS_TASK_SETPGID` — stub (not yet implemented).
pub fn sys_task_setpgid(_pid: usize, _pgid: usize) -> isize {
    -ENOSYS
}

/// `SYS_TASK_GETPGID` — stub (not yet implemented).
pub fn sys_task_getpgid(_pid: usize) -> isize {
    -ENOSYS
}

/// `SYS_TASK_GETCWD(buf_ptr, buf_len)` — get current working directory.
///
/// Copies the CWD path into the user buffer. Returns the path length on
/// success (excluding any terminator), or a negative error code.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "path length fits in isize on x86_64"
)]
pub fn sys_task_getcwd(buf_ptr: usize, buf_len: usize) -> isize {
    let cwd = match process::with_current_process(|p| p.cwd()) {
        Some(c) => c,
        None => return -ESRCH,
    };

    if cwd.len() > buf_len {
        return -ERANGE;
    }

    let buf = match UserPtrMut::<u8>::new(buf_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };

    // SAFETY: buf_ptr was validated and buf_len covers the CWD string.
    unsafe {
        core::ptr::copy_nonoverlapping(cwd.as_ptr(), buf.as_mut_ptr(), cwd.len());
    }

    cwd.len() as isize
}

/// `SYS_TASK_CHDIR(path_ptr, path_len)` — change working directory.
///
/// Updates the process CWD to the given absolute path. For Phase 5 this
/// does a simple validation (path must start with `/`) without verifying
/// the directory exists via VFS. Full validation via vnode_open with
/// `OPEN_DIRECTORY` is deferred to a later phase.
pub fn sys_task_chdir(path_ptr: usize, path_len: usize) -> isize {
    let path_slice = match UserSlice::new(path_ptr, path_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: User buffer was range-validated.
    let path_bytes = unsafe { path_slice.read_to_vec() };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(p) => p,
        Err(_) => return -EINVAL,
    };

    // Must be an absolute path.
    if !path.starts_with('/') {
        return -EINVAL;
    }

    let normalized = crate::vfs::normalize_path(path);

    process::with_current_process(|p| {
        p.set_cwd(normalized);
    });

    0
}

/// Frame deallocation callback for `AddressSpace::new_user`.
fn dealloc_frame(frame: PhysFrame<Size4KiB>) {
    hadron_mm::pmm::with(|pmm| {
        // SAFETY: The frame was allocated by us and is no longer mapped.
        unsafe {
            let _ = pmm.deallocate_frame(frame);
        }
    });
}
