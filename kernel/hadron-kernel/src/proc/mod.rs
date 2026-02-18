//! Process management.
//!
//! Each process is an async task on the executor. The process task calls
//! [`enter_userspace`] which saves kernel context, does `iretq` to ring 3,
//! and "returns" when a syscall or fault invokes [`restore_kernel_context`].

pub mod binfmt;
pub mod exec;

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use hadron_core::addr::PhysAddr;
use hadron_core::arch::x86_64::paging::PageTableMapper;
use hadron_core::arch::x86_64::registers::control::Cr3;
use hadron_core::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
use hadron_core::arch::x86_64::userspace::enter_userspace_save;
use hadron_core::mm::address_space::AddressSpace;
use hadron_core::sync::SpinLock;
use hadron_core::{kdebug, kinfo};

use crate::fs::file::{FileDescriptorTable, OpenFlags};

/// Saved kernel CR3 so that syscall/fault handlers can restore it.
static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);

/// Next PID to assign.
static NEXT_PID: AtomicU32 = AtomicU32::new(1);

/// Saved kernel RSP for `restore_kernel_context`.
/// Global for BSP-only; Phase 12 makes this per-CPU.
static SAVED_KERNEL_RSP: AtomicU64 = AtomicU64::new(0);

/// Exit status written by `sys_task_exit` / fault handler before restoring context.
/// `usize::MAX` is a sentinel meaning "killed by fault".
static PROCESS_EXIT_STATUS: AtomicU64 = AtomicU64::new(0);

/// The currently running user-mode process.
/// Set before entering userspace, cleared after returning.
/// BSP-only; Phase 12 makes this per-CPU.
static CURRENT_PROCESS: SpinLock<Option<Arc<Process>>> = SpinLock::new(None);

/// A user-mode process.
///
/// Owns an [`AddressSpace`] which is freed automatically on drop via
/// the stored deallocation callback.
pub struct Process {
    /// Process ID.
    pub pid: u32,
    /// Physical address of the user PML4 (cached for fast CR3 switch).
    pub user_cr3: PhysAddr,
    /// User address space (owns the PML4, freed on drop).
    /// Held for its `Drop` impl — not read directly.
    #[allow(dead_code, reason = "held for RAII cleanup in Drop")]
    address_space: AddressSpace<PageTableMapper>,
    /// Per-process file descriptor table.
    pub fd_table: SpinLock<FileDescriptorTable>,
}

impl Process {
    /// Creates a new process with the given address space.
    pub fn new(address_space: AddressSpace<PageTableMapper>) -> Self {
        let user_cr3 = address_space.root_phys();
        Self {
            pid: NEXT_PID.fetch_add(1, Ordering::Relaxed),
            user_cr3,
            address_space,
            fd_table: SpinLock::new(FileDescriptorTable::new()),
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        kdebug!(
            "Process {}: dropping (address space will be freed)",
            self.pid
        );
        // AddressSpace::Drop fires automatically, freeing the PML4 frame
        // via the dealloc_fn stored at construction time.
    }
}

/// The result of returning from userspace.
pub enum UserspaceReturn {
    /// Process exited via `sys_task_exit`.
    Syscall(SyscallInfo),
    // Preempted — Phase 9
    // Fault — future extension
}

/// Information about the syscall that caused userspace to return.
pub struct SyscallInfo {
    /// The exit status passed to `sys_task_exit`.
    pub exit_status: usize,
}

/// Returns the saved kernel CR3 physical address.
pub fn kernel_cr3() -> PhysAddr {
    PhysAddr::new(KERNEL_CR3.load(Ordering::Acquire))
}

/// Returns the saved kernel RSP for `restore_kernel_context`.
pub fn saved_kernel_rsp() -> u64 {
    SAVED_KERNEL_RSP.load(Ordering::Acquire)
}

/// Stores the exit status so `enter_userspace` can read it after context restore.
pub fn set_process_exit_status(status: u64) {
    PROCESS_EXIT_STATUS.store(status, Ordering::Release);
}

/// Saves the current CR3 as the kernel CR3.
pub fn save_kernel_cr3() {
    KERNEL_CR3.store(Cr3::read().as_u64(), Ordering::Release);
}

/// Execute a closure with a reference to the current process.
///
/// Called from syscall handlers to access the running process's state
/// (e.g. fd table).
///
/// # Panics
///
/// Panics if no process is currently running.
pub fn with_current_process<R>(f: impl FnOnce(&Arc<Process>) -> R) -> R {
    let guard = CURRENT_PROCESS.lock();
    let process = guard.as_ref().expect("no current process");
    f(process)
}

/// Enters userspace for the given process, returning when a syscall or
/// fault calls `restore_kernel_context`.
///
/// Disables interrupts, sets up GS bases for user/kernel transition,
/// switches CR3, and calls `enter_userspace_save`. Returns a
/// [`UserspaceReturn`] describing why userspace exited.
fn enter_userspace(process: &Process, entry: u64, stack_top: u64) -> UserspaceReturn {
    // Disable interrupts for the CR3/GS manipulation.
    // iretq re-enables them via RFLAGS.IF in userspace.
    // SAFETY: CLI has no side effects beyond masking interrupts. We need
    // interrupts off to atomically switch CR3 and GS bases.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    // SAFETY: Reading IA32_GS_BASE is safe; the MSR contains the current
    // per-CPU data pointer set during boot.
    let percpu_addr = unsafe { IA32_GS_BASE.read() };
    // SAFETY: We are preparing for iretq to userspace. Setting KERNEL_GS_BASE
    // to percpu_addr means swapgs in the syscall entry stub will restore it.
    // Setting GS_BASE to 0 gives user code a zeroed GS. Switching CR3 to the
    // user page table is safe because the kernel upper half is identity-mapped
    // in both address spaces. The saved_rsp_ptr points to a static AtomicU64,
    // and the assembly writes the current RSP there before entering userspace.
    unsafe {
        IA32_KERNEL_GS_BASE.write(percpu_addr);
        IA32_GS_BASE.write(0);

        // Switch to user address space.
        Cr3::write(process.user_cr3);

        // Pass a pointer directly into the SAVED_KERNEL_RSP static.
        // The assembly writes *saved_rsp_ptr = rsp BEFORE iretq, so the
        // value is available to syscall/fault handlers while userspace runs.
        // AtomicU64 has the same layout as u64, so this cast is sound.
        let saved_rsp_ptr = core::ptr::addr_of!(SAVED_KERNEL_RSP) as *mut u64;
        enter_userspace_save(entry, stack_top, saved_rsp_ptr);
    }

    // We're back from userspace (restore_kernel_context jumped us here).
    // CR3 and GS were already restored by the syscall/fault handler.
    let status = PROCESS_EXIT_STATUS.load(Ordering::Acquire);
    UserspaceReturn::Syscall(SyscallInfo {
        exit_status: status as usize,
    })
}

/// Loads and spawns the init process from the VFS.
///
/// Reads the `/init` binary from the mounted root filesystem, creates a
/// process, and sets up fd 0/1/2 pointing to `/dev/console`.
///
/// # Panics
///
/// Panics if `/init` does not exist in the VFS or if the ELF binary
/// cannot be loaded.
pub fn spawn_init() {
    let init_elf = read_init_from_vfs();

    let (process, entry, stack_top) =
        exec::create_process_from_binary(init_elf).expect("failed to load init binary");

    // Set up stdin/stdout/stderr pointing to /dev/console.
    {
        let console = crate::fs::vfs::with_vfs(|vfs| {
            vfs.resolve("/dev/console")
                .expect("spawn_init: /dev/console not found")
        });
        let mut fd_table = process.fd_table.lock();
        fd_table.insert_at(0, console.clone(), OpenFlags::READ);
        fd_table.insert_at(1, console.clone(), OpenFlags::WRITE);
        fd_table.insert_at(2, console, OpenFlags::WRITE);
    }

    let process = Arc::new(process);

    kinfo!(
        "Process {}: spawning init task (entry={:#x}, stack={:#x})",
        process.pid,
        entry,
        stack_top
    );

    crate::sched::spawn(process_task(process, entry, stack_top));
}

/// Reads the `/init` binary from the VFS.
///
/// Returns the file data as a leaked byte slice (lives for the kernel's lifetime).
fn read_init_from_vfs() -> &'static [u8] {
    use crate::fs::{poll_immediate, vfs};

    let inode = vfs::with_vfs(|vfs| vfs.resolve("/init").expect("VFS does not contain /init"));

    let file_size = inode.size();
    kinfo!("Found /init in VFS: {} bytes", file_size);

    let mut buf = alloc::vec![0u8; file_size];
    let bytes_read =
        poll_immediate(inode.read(0, &mut buf)).expect("failed to read /init from VFS");
    assert_eq!(bytes_read, file_size, "short read of /init");

    // Leak the Vec to get a 'static slice — we only load one init
    // binary and it must live for the kernel's lifetime.
    buf.leak()
}

/// The async task that represents a running process.
///
/// Calls `enter_userspace` and handles the return. Currently the process
/// runs once (no re-entry loop); future phases will add signal delivery
/// and re-entry for preempted processes.
async fn process_task(process: Arc<Process>, entry: u64, stack_top: u64) {
    let pid = process.pid;

    // Set the current process so syscall handlers can access it.
    {
        let mut current = CURRENT_PROCESS.lock();
        *current = Some(process.clone());
    }

    let result = enter_userspace(&process, entry, stack_top);

    // Clear current process.
    {
        let mut current = CURRENT_PROCESS.lock();
        *current = None;
    }

    match result {
        UserspaceReturn::Syscall(info) => {
            if info.exit_status == usize::MAX {
                kinfo!("Process {} killed by fault", pid);
            } else {
                kinfo!("Process {} exited with status {}", pid, info.exit_status);
            }
        }
    }

    // Process drops here — address space is freed.
    drop(process);
}
