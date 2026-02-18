//! Process management.
//!
//! Each process is an async task on the executor. The process task calls
//! [`enter_userspace_first`] on initial entry, which saves kernel context,
//! does `iretq` to ring 3, and "returns" when a syscall, fault, or timer
//! preemption invokes [`restore_kernel_context`]. Preempted processes are
//! re-entered via [`enter_userspace_resume_wrapper`] using saved register
//! state.

pub mod binfmt;
pub mod exec;

extern crate alloc;

use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};

use hadron_core::addr::PhysAddr;
use hadron_core::arch::x86_64::paging::PageTableMapper;
use hadron_core::arch::x86_64::registers::control::Cr3;
use hadron_core::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
use hadron_core::arch::x86_64::userspace::{
    UserRegisters, enter_userspace_resume, enter_userspace_save, restore_kernel_context,
};
use hadron_core::mm::address_space::AddressSpace;
use hadron_core::sync::SpinLock;
use hadron_core::{kdebug, kinfo};

use crate::fs::file::{FileDescriptorTable, OpenFlags};

// ── Trap reason constants ────────────────────────────────────────────

/// Userspace returned via `sys_task_exit`.
pub const TRAP_EXIT: u8 = 0;
/// Userspace was preempted by the timer interrupt.
pub const TRAP_PREEMPTED: u8 = 1;
/// Userspace was killed by a fault.
pub const TRAP_FAULT: u8 = 2;

// ── Global statics ──────────────────────────────────────────────────

/// Saved kernel CR3 so that syscall/fault handlers can restore it.
/// `pub(crate)` for access from the timer preemption stub.
pub(crate) static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);

/// Next PID to assign.
static NEXT_PID: AtomicU32 = AtomicU32::new(1);

/// Saved kernel RSP for `restore_kernel_context`.
/// `pub(crate)` for access from the timer preemption stub.
/// Global for BSP-only; Phase 12 makes this per-CPU.
pub(crate) static SAVED_KERNEL_RSP: AtomicU64 = AtomicU64::new(0);

/// Exit status written by `sys_task_exit` / fault handler before restoring context.
/// `usize::MAX` is a sentinel meaning "killed by fault".
static PROCESS_EXIT_STATUS: AtomicU64 = AtomicU64::new(0);

/// The currently running user-mode process.
/// Set before entering userspace, cleared after returning.
/// BSP-only; Phase 12 makes this per-CPU.
static CURRENT_PROCESS: SpinLock<Option<Arc<Process>>> = SpinLock::new(None);

/// Wrapper to make `UnsafeCell<UserRegisters>` usable in a `static`.
///
/// # Safety
///
/// Only accessed single-threaded: from the BSP's process task and from
/// the timer preemption stub (which runs with interrupts disabled while
/// userspace was executing, so the process task is suspended). Phase 12
/// will make this per-CPU.
pub(crate) struct SyncUserContext(UnsafeCell<UserRegisters>);

// SAFETY: See SyncUserContext doc comment — no concurrent access.
unsafe impl Sync for SyncUserContext {}

impl SyncUserContext {
    const fn new() -> Self {
        Self(UnsafeCell::new(UserRegisters {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            rsp: 0,
            rflags: 0,
        }))
    }

    fn get(&self) -> *mut UserRegisters {
        self.0.get()
    }
}

/// Saved user register state. Written by the preemption stub when
/// preempting from ring 3. Read by `enter_userspace_resume` when
/// re-entering. BSP-only; Phase 12 makes this per-CPU.
pub(crate) static USER_CONTEXT: SyncUserContext = SyncUserContext::new();

/// Why userspace execution stopped. Set before `restore_kernel_context`.
/// `pub(crate)` for access from the timer preemption stub.
/// BSP-only; Phase 12 makes this per-CPU.
pub(crate) static TRAP_REASON: AtomicU8 = AtomicU8::new(TRAP_EXIT);

// ── Process struct ──────────────────────────────────────────────────

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

// ── Public accessors ────────────────────────────────────────────────

/// Returns the saved kernel CR3 physical address.
pub fn kernel_cr3() -> PhysAddr {
    PhysAddr::new(KERNEL_CR3.load(Ordering::Acquire))
}

/// Returns the saved kernel RSP for `restore_kernel_context`.
pub fn saved_kernel_rsp() -> u64 {
    SAVED_KERNEL_RSP.load(Ordering::Acquire)
}

/// Stores the exit status so `process_task` can read it after context restore.
pub fn set_process_exit_status(status: u64) {
    PROCESS_EXIT_STATUS.store(status, Ordering::Release);
}

/// Saves the current CR3 as the kernel CR3.
pub fn save_kernel_cr3() {
    KERNEL_CR3.store(Cr3::read().as_u64(), Ordering::Release);
}

/// Sets the trap reason before calling `restore_kernel_context`.
pub fn set_trap_reason(reason: u8) {
    TRAP_REASON.store(reason, Ordering::Release);
}

/// Returns a raw pointer to the `USER_CONTEXT` static.
///
/// Used by the timer preemption stub to save user registers.
///
/// # Safety
///
/// The caller must ensure exclusive access (interrupts disabled or
/// single-threaded context).
pub fn user_context_ptr() -> *mut UserRegisters {
    USER_CONTEXT.get()
}

/// Returns a raw pointer to the `SAVED_KERNEL_RSP` static.
///
/// Used by assembly stubs to read the saved RSP value.
///
/// # Safety
///
/// The pointer must only be dereferenced in contexts where the value
/// is valid (after `enter_userspace_save` has stored a value).
pub fn saved_kernel_rsp_ptr() -> *const u64 {
    // AtomicU64 has the same layout as u64.
    core::ptr::addr_of!(SAVED_KERNEL_RSP) as *const u64
}

/// Returns a raw pointer to the `KERNEL_CR3` static.
///
/// Used by the timer preemption stub to restore kernel CR3.
pub fn kernel_cr3_ptr() -> *const u64 {
    core::ptr::addr_of!(KERNEL_CR3) as *const u64
}

/// Terminates the current user process due to a fault.
///
/// Restores kernel CR3 and GS bases, sets the fault trap reason,
/// then calls [`restore_kernel_context`] to longjmp back to `process_task`.
/// Called from exception handlers when a fault originates from ring 3.
///
/// # Safety
///
/// Must only be called from an interrupt/exception context where a user
/// process is running and `SAVED_KERNEL_RSP` contains a valid saved RSP
/// from `enter_userspace_save`.
pub unsafe fn terminate_current_process_from_fault() -> ! {
    // SAFETY: Restoring kernel CR3 is safe because the kernel upper half
    // is identity-mapped in the user address space.
    unsafe {
        Cr3::write(kernel_cr3());
    }
    // SAFETY: Reading KERNEL_GS_BASE gives us the percpu pointer that was
    // saved before entering userspace. Writing it to GS_BASE restores the
    // kernel's expected GS state.
    unsafe {
        let percpu = IA32_KERNEL_GS_BASE.read();
        IA32_GS_BASE.write(percpu);
    }
    set_process_exit_status(usize::MAX as u64);
    set_trap_reason(TRAP_FAULT);
    // SAFETY: saved_kernel_rsp() returns the RSP saved by enter_userspace_save,
    // which is still valid on the executor stack.
    unsafe {
        restore_kernel_context(saved_kernel_rsp());
    }
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

// ── Userspace entry helpers ─────────────────────────────────────────

/// Enters userspace for the first time (initial entry).
///
/// Disables interrupts, sets up GS bases for user/kernel transition,
/// switches CR3, and calls `enter_userspace_save`.
fn enter_userspace_first(process: &Process, entry: u64, stack_top: u64) {
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
    // in both address spaces.
    unsafe {
        IA32_KERNEL_GS_BASE.write(percpu_addr);
        IA32_GS_BASE.write(0);

        // Switch to user address space.
        Cr3::write(process.user_cr3);

        // Pass a pointer directly into the SAVED_KERNEL_RSP static.
        // AtomicU64 has the same layout as u64, so this cast is sound.
        let saved_rsp_ptr = core::ptr::addr_of!(SAVED_KERNEL_RSP) as *mut u64;
        enter_userspace_save(entry, stack_top, saved_rsp_ptr);
    }
    // Returns here when restore_kernel_context is called.
}

/// Re-enters userspace from saved register state (after preemption).
///
/// Disables interrupts, sets up GS bases, switches CR3, and calls
/// `enter_userspace_resume` with the saved `USER_CONTEXT`.
fn enter_userspace_resume_wrapper(process: &Process) {
    // SAFETY: CLI to mask interrupts during CR3/GS manipulation.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    let percpu_addr = unsafe { IA32_GS_BASE.read() };
    unsafe {
        IA32_KERNEL_GS_BASE.write(percpu_addr);
        IA32_GS_BASE.write(0);
        Cr3::write(process.user_cr3);

        let saved_rsp_ptr = core::ptr::addr_of!(SAVED_KERNEL_RSP) as *mut u64;
        // SAFETY: USER_CONTEXT was written by the timer preemption stub
        // and contains the user's register state at the point of preemption.
        // No one else accesses it between the timer stub and here.
        let ctx = USER_CONTEXT.get();
        enter_userspace_resume(ctx, saved_rsp_ptr);
    }
    // Returns here when restore_kernel_context is called.
}

// ── Process lifecycle ───────────────────────────────────────────────

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
/// Enters userspace and re-enters after preemption in a loop.
/// Exits when the process calls `sys_task_exit` or is killed by a fault.
async fn process_task(process: Arc<Process>, entry: u64, stack_top: u64) {
    let pid = process.pid;
    let mut first_entry = true;

    loop {
        // Set the current process so syscall handlers can access it.
        {
            let mut current = CURRENT_PROCESS.lock();
            *current = Some(process.clone());
        }

        if first_entry {
            first_entry = false;
            enter_userspace_first(&process, entry, stack_top);
        } else {
            enter_userspace_resume_wrapper(&process);
        }

        // We're back from userspace. CR3 and GS were already restored
        // by the syscall/fault/preemption handler.
        {
            let mut current = CURRENT_PROCESS.lock();
            *current = None;
        }

        match TRAP_REASON.load(Ordering::Acquire) {
            TRAP_EXIT => {
                let status = PROCESS_EXIT_STATUS.load(Ordering::Acquire);
                if status == usize::MAX as u64 {
                    kinfo!("Process {} killed by fault", pid);
                } else {
                    kinfo!("Process {} exited with status {}", pid, status);
                }
                break;
            }
            TRAP_PREEMPTED => {
                // User state was saved in USER_CONTEXT by the timer stub.
                // Yield to the executor so other tasks can run, then
                // re-enter userspace on next poll.
                crate::sched::primitives::yield_now().await;
                continue;
            }
            TRAP_FAULT => {
                kinfo!("Process {} killed by fault", pid);
                break;
            }
            _ => unreachable!("invalid trap reason"),
        }
    }

    // Process drops here — address space is freed.
    drop(process);
}
