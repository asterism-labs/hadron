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

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use crate::addr::{PhysAddr, VirtAddr};
use crate::arch::x86_64::paging::PageTableMapper;
use crate::arch::x86_64::registers::control::Cr3;
use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
use crate::arch::x86_64::userspace::{
    UserRegisters, enter_userspace_resume, enter_userspace_save, restore_kernel_context,
};
use crate::mm::address_space::AddressSpace;
use crate::mm::layout::VirtRegion;
use crate::mm::region::FreeRegionAllocator;
use crate::percpu::{CpuLocal, MAX_CPUS};
use crate::sync::SpinLock;
use crate::{kdebug, kinfo};

use crate::fs::file::{FileDescriptorTable, OpenFlags};
use crate::sync::HeapWaitQueue;

// ── Trap reason constants ────────────────────────────────────────────

/// Userspace returned via `sys_task_exit`.
pub const TRAP_EXIT: u8 = 0;
/// Userspace was preempted by the timer interrupt.
pub const TRAP_PREEMPTED: u8 = 1;
/// Userspace was killed by a fault.
pub const TRAP_FAULT: u8 = 2;
/// Syscall requested blocking wait (sys_task_wait).
pub const TRAP_WAIT: u8 = 3;
/// Syscall requested blocking I/O (pipe read/write).
pub const TRAP_IO: u8 = 4;

// ── Global statics ──────────────────────────────────────────────────

/// Saved kernel CR3 so that syscall/fault handlers can restore it.
/// `pub(crate)` for access from the timer preemption stub.
pub(crate) static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);

/// Next PID to assign.
static NEXT_PID: AtomicU32 = AtomicU32::new(1);

/// Per-CPU saved kernel RSP for `restore_kernel_context`.
/// `pub(crate)` for access from the timer preemption stub.
pub(crate) static SAVED_KERNEL_RSP: CpuLocal<AtomicU64> =
    CpuLocal::new([const { AtomicU64::new(0) }; MAX_CPUS]);

/// Per-CPU exit status written by `sys_task_exit` / fault handler before restoring context.
/// `usize::MAX` is a sentinel meaning "killed by fault".
static PROCESS_EXIT_STATUS: CpuLocal<AtomicU64> =
    CpuLocal::new([const { AtomicU64::new(0) }; MAX_CPUS]);

/// Per-CPU currently running user-mode process.
/// Set before entering userspace, cleared after returning.
static CURRENT_PROCESS: CpuLocal<SpinLock<Option<Arc<Process>>>> =
    CpuLocal::new([const { SpinLock::new(None) }; MAX_CPUS]);

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

    pub(crate) fn get(&self) -> *mut UserRegisters {
        self.0.get()
    }
}

/// Per-CPU saved user register state. Written by the preemption stub when
/// preempting from ring 3. Read by `enter_userspace_resume` when
/// re-entering.
pub(crate) static USER_CONTEXT: CpuLocal<SyncUserContext> =
    CpuLocal::new([const { SyncUserContext::new() }; MAX_CPUS]);

/// Per-CPU trap reason. Set before `restore_kernel_context`.
/// `pub(crate)` for access from the timer preemption stub.
pub(crate) static TRAP_REASON: CpuLocal<AtomicU8> =
    CpuLocal::new([const { AtomicU8::new(TRAP_EXIT) }; MAX_CPUS]);

/// Per-CPU target PID for `sys_task_wait`. Set by syscall handler, read by `process_task`.
static WAIT_TARGET_PID: CpuLocal<AtomicU32> =
    CpuLocal::new([const { AtomicU32::new(0) }; MAX_CPUS]);

/// Per-CPU user-space pointer where `sys_task_wait` should write the exit status.
static WAIT_STATUS_PTR: CpuLocal<AtomicU64> =
    CpuLocal::new([const { AtomicU64::new(0) }; MAX_CPUS]);

/// Per-CPU file descriptor for TRAP_IO.
static IO_FD: CpuLocal<AtomicU64> = CpuLocal::new([const { AtomicU64::new(0) }; MAX_CPUS]);

/// Per-CPU user buffer pointer for TRAP_IO.
static IO_BUF_PTR: CpuLocal<AtomicU64> = CpuLocal::new([const { AtomicU64::new(0) }; MAX_CPUS]);

/// Per-CPU user buffer length for TRAP_IO.
static IO_BUF_LEN: CpuLocal<AtomicU64> = CpuLocal::new([const { AtomicU64::new(0) }; MAX_CPUS]);

/// Per-CPU I/O direction for TRAP_IO: 0 = read, 1 = write.
static IO_IS_WRITE: CpuLocal<AtomicU8> = CpuLocal::new([const { AtomicU8::new(0) }; MAX_CPUS]);

// ── Global process table ────────────────────────────────────────────

/// Global process table mapping PID → `Arc<Process>`.
///
/// Processes are inserted on spawn and removed after exit + reaping.
static PROCESS_TABLE: SpinLock<BTreeMap<u32, Arc<Process>>> = SpinLock::new(BTreeMap::new());

/// Registers a process in the global table.
pub fn register_process(process: &Arc<Process>) {
    let mut table = PROCESS_TABLE.lock();
    table.insert(process.pid, process.clone());
}

/// Looks up a process by PID.
pub fn lookup_process(pid: u32) -> Option<Arc<Process>> {
    let table = PROCESS_TABLE.lock();
    table.get(&pid).cloned()
}

/// Removes a process from the global table.
pub fn unregister_process(pid: u32) {
    let mut table = PROCESS_TABLE.lock();
    table.remove(&pid);
}

/// Returns the PIDs of all children of the given parent.
pub fn children_of(parent_pid: u32) -> Vec<u32> {
    let table = PROCESS_TABLE.lock();
    table
        .values()
        .filter(|p| p.parent_pid == Some(parent_pid))
        .map(|p| p.pid)
        .collect()
}

// ── User mmap region ────────────────────────────────────────────────

/// Base address for user mmap allocations (middle of lower-half address space).
const USER_MMAP_BASE: u64 = 0x0000_4000_0000_0000;

/// Maximum size of the user mmap region: 256 TiB.
const USER_MMAP_MAX_SIZE: u64 = 256 * 1024 * 1024 * 1024 * 1024;

/// Maximum free-list entries for the per-process mmap allocator.
const MMAP_FREE_LIST_CAPACITY: usize = 64;

// ── Process struct ──────────────────────────────────────────────────

/// A user-mode process.
///
/// Owns an [`AddressSpace`] which is freed automatically on drop via
/// the stored deallocation callback.
pub struct Process {
    /// Process ID.
    pub pid: u32,
    /// Parent process ID (`None` for init).
    pub parent_pid: Option<u32>,
    /// Physical address of the user PML4 (cached for fast CR3 switch).
    pub user_cr3: PhysAddr,
    /// User address space (owns the PML4, freed on drop).
    /// Held for its `Drop` impl — not read directly.
    #[allow(dead_code, reason = "held for RAII cleanup in Drop")]
    address_space: AddressSpace<PageTableMapper>,
    /// Per-process file descriptor table.
    pub fd_table: SpinLock<FileDescriptorTable>,
    /// Virtual address region allocator for `sys_mem_map` mappings.
    pub(crate) mmap_alloc: SpinLock<FreeRegionAllocator<MMAP_FREE_LIST_CAPACITY>>,
    /// Exit status, set when the process terminates.
    pub exit_status: SpinLock<Option<u64>>,
    /// Wait queue notified when this process exits.
    pub exit_notify: HeapWaitQueue,
}

impl Process {
    /// Returns a reference to the process's address space.
    pub(crate) fn address_space(&self) -> &AddressSpace<PageTableMapper> {
        &self.address_space
    }

    /// Creates a new process with the given address space and parent PID.
    pub fn new(address_space: AddressSpace<PageTableMapper>, parent_pid: Option<u32>) -> Self {
        let user_cr3 = address_space.root_phys();
        let mmap_region =
            VirtRegion::new(VirtAddr::new(USER_MMAP_BASE), USER_MMAP_MAX_SIZE);
        Self {
            pid: NEXT_PID.fetch_add(1, Ordering::Relaxed),
            parent_pid,
            user_cr3,
            address_space,
            fd_table: SpinLock::new(FileDescriptorTable::new()),
            mmap_alloc: SpinLock::new(FreeRegionAllocator::new(mmap_region)),
            exit_status: SpinLock::new(None),
            exit_notify: HeapWaitQueue::new(),
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

/// Returns the saved kernel RSP for `restore_kernel_context` (current CPU).
pub fn saved_kernel_rsp() -> u64 {
    SAVED_KERNEL_RSP.get().load(Ordering::Acquire)
}

/// Stores the exit status so `process_task` can read it after context restore (current CPU).
pub fn set_process_exit_status(status: u64) {
    PROCESS_EXIT_STATUS.get().store(status, Ordering::Release);
}

/// Saves the current CR3 as the kernel CR3.
pub fn save_kernel_cr3() {
    KERNEL_CR3.store(Cr3::read().as_u64(), Ordering::Release);
}

/// Sets the trap reason before calling `restore_kernel_context` (current CPU).
pub fn set_trap_reason(reason: u8) {
    TRAP_REASON.get().store(reason, Ordering::Release);
}

/// Returns a raw pointer to the current CPU's `USER_CONTEXT`.
///
/// Used by the timer preemption stub to save user registers.
///
/// # Safety
///
/// The caller must ensure exclusive access (interrupts disabled or
/// single-threaded context).
pub fn user_context_ptr() -> *mut UserRegisters {
    USER_CONTEXT.get().get()
}

/// Returns a raw pointer to the current CPU's `SAVED_KERNEL_RSP`.
///
/// Used by assembly stubs to read the saved RSP value.
///
/// # Safety
///
/// The pointer must only be dereferenced in contexts where the value
/// is valid (after `enter_userspace_save` has stored a value).
pub fn saved_kernel_rsp_ptr() -> *const u64 {
    // AtomicU64 has the same layout as u64.
    SAVED_KERNEL_RSP.get() as *const AtomicU64 as *const u64
}

/// Returns a raw pointer to the current CPU's `TRAP_REASON`.
///
/// Used by the timer preemption stub to set the trap reason.
pub fn trap_reason_ptr() -> *const u8 {
    TRAP_REASON.get() as *const AtomicU8 as *const u8
}

/// Returns a raw pointer to the `KERNEL_CR3` static (global, not per-CPU).
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
    PROCESS_EXIT_STATUS
        .get()
        .store(usize::MAX as u64, Ordering::Release);
    TRAP_REASON.get().store(TRAP_FAULT, Ordering::Release);
    // SAFETY: saved_kernel_rsp() returns the RSP saved by enter_userspace_save,
    // which is still valid on the executor stack.
    unsafe {
        restore_kernel_context(SAVED_KERNEL_RSP.get().load(Ordering::Acquire));
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
    let guard = CURRENT_PROCESS.get().lock();
    let process = guard.as_ref().expect("no current process");
    f(process)
}

/// Try to execute a closure with a reference to the current process.
///
/// Returns `None` if no process is currently running (e.g. in the test
/// harness or during early boot).
pub fn try_current_process<R>(f: impl FnOnce(&Arc<Process>) -> R) -> Option<R> {
    let guard = CURRENT_PROCESS.get().lock();
    guard.as_ref().map(f)
}

// ── Userspace entry helpers ─────────────────────────────────────────

/// Enters userspace for the first time (initial entry).
///
/// Disables interrupts, sets up GS bases for user/kernel transition,
/// switches CR3, and calls `enter_userspace_save`.
fn enter_userspace_first(process: &Process, entry: u64, stack_top: u64) {
    // Compute per-CPU pointer BEFORE clearing GS base (CpuLocal::get()
    // needs current_cpu() which reads GS:[0]).
    let saved_rsp_ptr = SAVED_KERNEL_RSP.get() as *const AtomicU64 as *mut u64;

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

        enter_userspace_save(entry, stack_top, saved_rsp_ptr);
    }
    // Returns here when restore_kernel_context is called.
}

/// Re-enters userspace from saved register state (after preemption).
///
/// Disables interrupts, sets up GS bases, switches CR3, and calls
/// `enter_userspace_resume` with the saved `USER_CONTEXT`.
fn enter_userspace_resume_wrapper(process: &Process) {
    // Compute per-CPU pointers BEFORE clearing GS base (CpuLocal::get()
    // needs current_cpu() which reads GS:[0]).
    let saved_rsp_ptr = SAVED_KERNEL_RSP.get() as *const AtomicU64 as *mut u64;
    let ctx = USER_CONTEXT.get().get();

    // SAFETY: CLI to mask interrupts during CR3/GS manipulation.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    let percpu_addr = unsafe { IA32_GS_BASE.read() };
    unsafe {
        IA32_KERNEL_GS_BASE.write(percpu_addr);
        IA32_GS_BASE.write(0);
        Cr3::write(process.user_cr3);

        // SAFETY: USER_CONTEXT was written by the timer preemption stub
        // and contains the user's register state at the point of preemption.
        // No one else accesses it between the timer stub and here.
        enter_userspace_resume(ctx, saved_rsp_ptr);
    }
    // Returns here when restore_kernel_context is called.
}

// ── Process lifecycle ───────────────────────────────────────────────

/// Loads and spawns the init process from the VFS.
///
/// Reads the `/bin/init` binary from the mounted root filesystem, creates a
/// process, and sets up fd 0/1/2 pointing to `/dev/console`.
///
/// # Panics
///
/// Panics if `/bin/init` does not exist in the VFS or if the ELF binary
/// cannot be loaded.
pub fn spawn_init() {
    let init_elf = read_init_from_vfs();

    let (process, entry, _stack_top) =
        exec::create_process_from_binary(init_elf, None).expect("failed to load init binary");

    // Write argv onto the init process's stack: ["/bin/init"].
    let hhdm_offset = crate::mm::hhdm::offset();
    let stack_top = exec::write_argv_to_init_stack(process.address_space(), hhdm_offset)
        .expect("failed to write argv for init");

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
    register_process(&process);

    kinfo!(
        "Process {}: spawning init task (entry={:#x}, stack={:#x})",
        process.pid,
        entry,
        stack_top
    );

    crate::sched::spawn(process_task(process, entry, stack_top));
}

/// Reads the `/bin/init` binary from the VFS.
///
/// Returns the file data as a leaked byte slice (lives for the kernel's lifetime).
fn read_init_from_vfs() -> &'static [u8] {
    use crate::fs::{poll_immediate, vfs};

    let inode =
        vfs::with_vfs(|vfs| vfs.resolve("/bin/init").expect("VFS does not contain /bin/init"));

    let file_size = inode.size();
    kinfo!("Found /bin/init in VFS: {} bytes", file_size);

    let mut buf = alloc::vec![0u8; file_size];
    let bytes_read =
        poll_immediate(inode.read(0, &mut buf)).expect("failed to read /bin/init from VFS");
    assert_eq!(bytes_read, file_size, "short read of /bin/init");

    // Leak the Vec to get a 'static slice — we only load one init
    // binary and it must live for the kernel's lifetime.
    buf.leak()
}

/// Sets the target PID and status pointer for a `TRAP_WAIT` syscall.
pub fn set_wait_params(target_pid: u32, status_ptr: u64) {
    WAIT_TARGET_PID.get().store(target_pid, Ordering::Release);
    WAIT_STATUS_PTR.get().store(status_ptr, Ordering::Release);
}

/// Sets the I/O parameters for a `TRAP_IO` syscall.
pub fn set_io_params(fd: usize, buf_ptr: usize, buf_len: usize, is_write: bool) {
    IO_FD.get().store(fd as u64, Ordering::Release);
    IO_BUF_PTR.get().store(buf_ptr as u64, Ordering::Release);
    IO_BUF_LEN.get().store(buf_len as u64, Ordering::Release);
    IO_IS_WRITE
        .get()
        .store(u8::from(is_write), Ordering::Release);
}

/// The async task that represents a running process.
///
/// Enters userspace and re-enters after preemption in a loop.
/// Exits when the process calls `sys_task_exit` or is killed by a fault.
pub(crate) async fn process_task(process: Arc<Process>, entry: u64, stack_top: u64) {
    let pid = process.pid;
    let mut first_entry = true;

    loop {
        // Set the current process so syscall handlers can access it.
        {
            let mut current = CURRENT_PROCESS.get().lock();
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
            let mut current = CURRENT_PROCESS.get().lock();
            *current = None;
        }

        match TRAP_REASON.get().load(Ordering::Acquire) {
            TRAP_EXIT => {
                let status = PROCESS_EXIT_STATUS.get().load(Ordering::Acquire);
                if status == usize::MAX as u64 {
                    kinfo!("Process {} killed by fault", pid);
                } else {
                    kinfo!("Process {} exited with status {}", pid, status);
                }
                // Store exit status and notify waiters.
                *process.exit_status.lock() = Some(status);
                process.exit_notify.wake_all();
                break;
            }
            TRAP_PREEMPTED => {
                // User state was saved in USER_CONTEXT by the timer stub.
                // Snapshot it before yielding — USER_CONTEXT is per-CPU but
                // will be overwritten if another process is preempted on
                // this CPU while we are suspended in .await.
                // SAFETY: USER_CONTEXT is only written by the timer stub
                // with interrupts disabled while userspace was running.
                // No concurrent access from here.
                let saved_ctx = unsafe { (*USER_CONTEXT.get().get()).clone() };

                // Yield to the executor so other tasks can run.
                crate::sched::primitives::yield_now().await;

                // Restore our saved context back before re-entering
                // userspace. Another process may have overwritten it
                // during our yield.
                // SAFETY: No other task accesses USER_CONTEXT between
                // here and enter_userspace_resume_wrapper (no .await).
                unsafe {
                    *USER_CONTEXT.get().get() = saved_ctx;
                }
                continue;
            }
            TRAP_FAULT => {
                kinfo!("Process {} killed by fault", pid);
                // Faults report as exit status usize::MAX.
                *process.exit_status.lock() = Some(usize::MAX as u64);
                process.exit_notify.wake_all();
                break;
            }
            TRAP_WAIT => {
                // The syscall handler set WAIT_TARGET_PID and WAIT_STATUS_PTR.
                // We await the child's exit here (async context).
                let target = WAIT_TARGET_PID.get().load(Ordering::Acquire);
                let status_ptr = WAIT_STATUS_PTR.get().load(Ordering::Acquire);

                // Snapshot the saved user registers and RSP BEFORE yielding.
                // SYSCALL_SAVED_REGS and percpu.user_rsp are global statics
                // that will be overwritten by the child process's syscalls
                // while we are suspended in .await.
                // SAFETY: SYSCALL_SAVED_REGS is only written by syscall entry
                // assembly with interrupts masked, and we haven't yielded yet.
                let (
                    saved_rip,
                    saved_rflags,
                    saved_rbx,
                    saved_rbp,
                    saved_r12,
                    saved_r13,
                    saved_r14,
                    saved_r15,
                    saved_user_rsp,
                ) = unsafe {
                    let saved = &*crate::arch::x86_64::syscall::SYSCALL_SAVED_REGS
                        .get()
                        .get();
                    (
                        saved.user_rip,
                        saved.user_rflags,
                        saved.rbx,
                        saved.rbp,
                        saved.r12,
                        saved.r13,
                        saved.r14,
                        saved.r15,
                        crate::percpu::current_cpu().user_rsp,
                    )
                };

                let (result, exit_code) = handle_wait(pid, target).await;

                // Write exit status to user memory under user CR3.
                if status_ptr != 0 && result >= 0 {
                    // SAFETY: Switching to user CR3 is safe because the kernel
                    // upper half is identity-mapped in both address spaces.
                    unsafe {
                        Cr3::write(process.user_cr3);
                    }
                    let uslice = crate::syscall::userptr::UserSlice::new(
                        status_ptr as usize,
                        core::mem::size_of::<u64>(),
                    );
                    if let Ok(slice) = uslice {
                        // SAFETY: The pointer was validated by UserSlice and the
                        // user address space is mapped (we switched CR3).
                        unsafe {
                            core::ptr::write(slice.addr() as *mut u64, exit_code);
                        }
                    }
                    // SAFETY: Restore kernel CR3.
                    unsafe {
                        Cr3::write(kernel_cr3());
                    }
                }

                // Populate USER_CONTEXT from the snapshotted registers.
                // SAFETY: USER_CONTEXT is per-CPU, only accessed from this
                // task and the preemption stub (mutually exclusive).
                unsafe {
                    let ctx = &mut *USER_CONTEXT.get().get();
                    ctx.rip = saved_rip;
                    ctx.rflags = saved_rflags;
                    ctx.rsp = saved_user_rsp;
                    ctx.rbx = saved_rbx;
                    ctx.rbp = saved_rbp;
                    ctx.r12 = saved_r12;
                    ctx.r13 = saved_r13;
                    ctx.r14 = saved_r14;
                    ctx.r15 = saved_r15;
                    ctx.rax = result as u64;
                    // Caller-saved registers are clobbered by the syscall ABI.
                    ctx.rcx = 0;
                    ctx.rdx = 0;
                    ctx.rsi = 0;
                    ctx.rdi = 0;
                    ctx.r8 = 0;
                    ctx.r9 = 0;
                    ctx.r10 = 0;
                    ctx.r11 = 0;
                }
                continue;
            }
            TRAP_IO => {
                // The syscall handler set IO_FD, IO_BUF_PTR, IO_BUF_LEN, IO_IS_WRITE.
                // Perform the async I/O here where we can .await.
                let io_fd = IO_FD.get().load(Ordering::Acquire) as usize;
                let io_buf_ptr = IO_BUF_PTR.get().load(Ordering::Acquire) as usize;
                let io_buf_len = IO_BUF_LEN.get().load(Ordering::Acquire) as usize;
                let is_write = IO_IS_WRITE.get().load(Ordering::Acquire) != 0;

                // Snapshot saved user registers (same pattern as TRAP_WAIT).
                // SAFETY: SYSCALL_SAVED_REGS is only written by syscall entry
                // assembly with interrupts masked, and we haven't yielded yet.
                let (
                    saved_rip,
                    saved_rflags,
                    saved_rbx,
                    saved_rbp,
                    saved_r12,
                    saved_r13,
                    saved_r14,
                    saved_r15,
                    saved_user_rsp,
                ) = unsafe {
                    let saved = &*crate::arch::x86_64::syscall::SYSCALL_SAVED_REGS
                        .get()
                        .get();
                    (
                        saved.user_rip,
                        saved.user_rflags,
                        saved.rbx,
                        saved.rbp,
                        saved.r12,
                        saved.r13,
                        saved.r14,
                        saved.r15,
                        crate::percpu::current_cpu().user_rsp,
                    )
                };

                // Get inode from the process fd table.
                let io_result = {
                    let fd_table = process.fd_table.lock();
                    fd_table
                        .get(io_fd)
                        .map(|f| (f.inode.clone(), f.offset, f.flags))
                };

                let result: isize = if let Some((inode, offset, flags)) = io_result {
                    if is_write {
                        if !flags.contains(crate::fs::file::OpenFlags::WRITE) {
                            -crate::syscall::EBADF
                        } else {
                            // Copy user data to kernel buffer under user CR3.
                            let mut kbuf = alloc::vec![0u8; io_buf_len];
                            // SAFETY: Switching to user CR3 to copy data.
                            unsafe {
                                Cr3::write(process.user_cr3);
                            }
                            let uslice = crate::syscall::userptr::UserSlice::new(
                                io_buf_ptr, io_buf_len,
                            );
                            if let Ok(slice) = uslice {
                                // SAFETY: UserSlice validated pointer range under user CR3.
                                let src = unsafe { slice.as_slice() };
                                kbuf.copy_from_slice(src);
                            }
                            // SAFETY: Restore kernel CR3.
                            unsafe {
                                Cr3::write(kernel_cr3());
                            }

                            match inode.write(offset, &kbuf).await {
                                Ok(n) => {
                                    let mut fd_table = process.fd_table.lock();
                                    if let Some(f) = fd_table.get_mut(io_fd) {
                                        f.offset += n;
                                    }
                                    #[expect(
                                        clippy::cast_possible_wrap,
                                        reason = "byte counts are small"
                                    )]
                                    {
                                        n as isize
                                    }
                                }
                                Err(e) => -e.to_errno(),
                            }
                        }
                    } else {
                        if !flags.contains(crate::fs::file::OpenFlags::READ) {
                            -crate::syscall::EBADF
                        } else {
                            let mut kbuf = alloc::vec![0u8; io_buf_len];
                            match inode.read(offset, &mut kbuf).await {
                                Ok(n) => {
                                    // Copy kernel buffer to user memory under user CR3.
                                    // SAFETY: Switching to user CR3.
                                    unsafe {
                                        Cr3::write(process.user_cr3);
                                    }
                                    let uslice = crate::syscall::userptr::UserSlice::new(
                                        io_buf_ptr, n,
                                    );
                                    if let Ok(slice) = uslice {
                                        // SAFETY: UserSlice validated pointer range under user CR3.
                                        let dst = unsafe { slice.as_mut_slice() };
                                        dst.copy_from_slice(&kbuf[..n]);
                                    }
                                    // SAFETY: Restore kernel CR3.
                                    unsafe {
                                        Cr3::write(kernel_cr3());
                                    }
                                    let mut fd_table = process.fd_table.lock();
                                    if let Some(f) = fd_table.get_mut(io_fd) {
                                        f.offset += n;
                                    }
                                    #[expect(
                                        clippy::cast_possible_wrap,
                                        reason = "byte counts are small"
                                    )]
                                    {
                                        n as isize
                                    }
                                }
                                Err(e) => {
                                    // Ensure kernel CR3 is active.
                                    -e.to_errno()
                                }
                            }
                        }
                    }
                } else {
                    -crate::syscall::EBADF
                };

                // Restore user registers and set result in rax.
                // SAFETY: USER_CONTEXT is per-CPU, only accessed from this
                // task and the preemption stub (mutually exclusive).
                unsafe {
                    let ctx = &mut *USER_CONTEXT.get().get();
                    ctx.rip = saved_rip;
                    ctx.rflags = saved_rflags;
                    ctx.rsp = saved_user_rsp;
                    ctx.rbx = saved_rbx;
                    ctx.rbp = saved_rbp;
                    ctx.r12 = saved_r12;
                    ctx.r13 = saved_r13;
                    ctx.r14 = saved_r14;
                    ctx.r15 = saved_r15;
                    ctx.rax = result as u64;
                    ctx.rcx = 0;
                    ctx.rdx = 0;
                    ctx.rsi = 0;
                    ctx.rdi = 0;
                    ctx.r8 = 0;
                    ctx.r9 = 0;
                    ctx.r10 = 0;
                    ctx.r11 = 0;
                }
                continue;
            }
            _ => unreachable!("invalid trap reason"),
        }
    }

    // Process remains in the table as a zombie until reaped by waitpid.
    // The Arc in PROCESS_TABLE keeps the Process alive so handle_wait
    // can still look it up and read exit_status.
    drop(process);
}

/// Handles a `TRAP_WAIT` by awaiting the target child's exit.
///
/// Returns `(child_pid, exit_code)` on success, or `(-errno, 0)` on failure.
/// The caller is responsible for writing the exit status to user memory
/// (requires switching to user CR3).
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning negated errno as isize"
)]
async fn handle_wait(parent_pid: u32, target_pid: u32) -> (isize, u64) {
    // Find the child process.
    let child = if target_pid == 0 {
        // Wait for any child — pick the first one.
        let children = children_of(parent_pid);
        if children.is_empty() {
            return (-(crate::syscall::EINVAL), 0);
        }
        lookup_process(children[0])
    } else {
        lookup_process(target_pid)
    };

    let child = match child {
        Some(c) => c,
        None => return (-(crate::syscall::EINVAL), 0),
    };

    // Verify it's actually our child.
    if child.parent_pid != Some(parent_pid) {
        return (-(crate::syscall::EINVAL), 0);
    }

    // Wait for the child to exit (may already be done).
    loop {
        {
            let status = child.exit_status.lock();
            if let Some(exit_code) = *status {
                // Reap: remove child from the process table now that
                // the parent has collected the exit status.
                unregister_process(child.pid);
                return (child.pid as isize, exit_code);
            }
        }
        // Not exited yet — wait for notification.
        child.exit_notify.wait().await;
    }
}
