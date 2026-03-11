//! Process execution and scheduling.
//!
//! Provides [`process_task`], an async function spawned on the per-CPU executor
//! for each user process. Each poll either enters userspace for the first time
//! or re-enters after a blocking syscall completes.
//!
//! The flow:
//! 1. `process_task` calls [`enter_userspace_save`] (setjmp + iretq)
//! 2. User code runs; non-blocking syscalls return via `sysretq`
//! 3. A blocking syscall stores a [`BlockingOp`] and calls [`restore_kernel_context`]
//! 4. Execution "returns" from `enter_userspace_save` into `process_task`
//! 5. `process_task` processes the op (possibly `.await`ing), then re-enters via
//!    [`enter_userspace_resume`]

extern crate alloc;

use alloc::sync::Arc;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use hadron_core::cpu_local::CpuLocal;
use hadron_core::sync::SpinLock;
use hadron_mm::address_space::AddressSpace;
use hadron_objects::object::{KernelObject, Signals};
use hadron_objects::process::Process;
use hadron_objects::thread::{Thread, ThreadState};

use crate::arch::x86_64::paging::PageTableMapper;
use crate::arch::x86_64::syscall::SYSCALL_SAVED_REGS;
use crate::arch::x86_64::userspace::{self, UserRegisters};
use crate::percpu::MAX_CPUS;

// ── Blocking operations ─────────────────────────────────────────────

/// Operations that cause a syscall handler to longjmp back to `process_task`.
pub enum BlockingOp {
    /// Process is exiting with the given code.
    Exit(usize),
    /// Waiting for a child process to terminate.
    TaskWait {
        /// Child process koid.
        pid: u64,
        /// User pointer to write exit status (0 = ignore).
        status_ptr: usize,
    },
}

/// Per-CPU pending blocking operation.
///
/// Set by a syscall handler before calling `restore_kernel_context`.
/// Read by `process_task` after returning from userspace.
static PENDING_OP: CpuLocal<SpinLock<Option<BlockingOp>>> =
    CpuLocal::new([const { SpinLock::new(None) }; MAX_CPUS]);

/// Store a blocking operation (called from syscall handlers before longjmp).
pub fn set_blocking_op(op: BlockingOp) {
    *PENDING_OP.get().lock() = Some(op);
}

/// Take the pending blocking operation.
fn take_blocking_op() -> Option<BlockingOp> {
    PENDING_OP.get().lock().take()
}

// ── Per-CPU current process context ─────────────────────────────────

/// Per-CPU current process reference.
static CURRENT_PROCESS: CpuLocal<SpinLock<Option<Arc<Process>>>> =
    CpuLocal::new([const { SpinLock::new(None) }; MAX_CPUS]);

/// Execute a closure with the current CPU's active process.
pub fn with_current_process<R>(f: impl FnOnce(&Arc<Process>) -> R) -> Option<R> {
    let guard = CURRENT_PROCESS.get().lock();
    guard.as_ref().map(f)
}

// ── Global process table ────────────────────────────────────────────

/// Global table mapping koid → process for task_wait lookups.
static PROCESS_TABLE: SpinLock<alloc::collections::BTreeMap<u64, Arc<Process>>> =
    SpinLock::new(alloc::collections::BTreeMap::new());

/// Register a process in the global table.
pub fn register_process(process: &Arc<Process>) {
    PROCESS_TABLE
        .lock()
        .insert(process.koid().raw(), Arc::clone(process));
}

/// Look up a process by koid.
pub fn lookup_process(koid: u64) -> Option<Arc<Process>> {
    PROCESS_TABLE.lock().get(&koid).cloned()
}

/// Remove a process from the global table.
pub fn unregister_process(koid: u64) {
    PROCESS_TABLE.lock().remove(&koid);
}

// ── Child exit waker (Phase 2b simple mechanism) ────────────────────

/// Per-CPU waker storage for a parent waiting on a child exit.
///
/// Phase 2b: only one waiter per CPU. Phase 2c replaces this with
/// proper observer-based waking via Port.
static CHILD_EXIT_WAKER: CpuLocal<SpinLock<Option<Waker>>> =
    CpuLocal::new([const { SpinLock::new(None) }; MAX_CPUS]);

/// Wake any parent waiting for a child exit on this CPU.
pub fn wake_child_exit_waiter() {
    if let Some(waker) = CHILD_EXIT_WAKER.get().lock().take() {
        waker.wake();
    }
}

// ── Process task ────────────────────────────────────────────────────

/// Snapshot of user-mode register state captured before yielding.
///
/// The per-CPU `SYSCALL_SAVED_REGS` and `percpu.user_rsp` are overwritten
/// when another process's syscall runs on this CPU. This struct captures
/// the values so they survive across `.await` points.
struct SavedUserState {
    /// User return RIP.
    user_rip: u64,
    /// User RFLAGS.
    user_rflags: u64,
    /// User RSP.
    user_rsp: u64,
    /// Callee-saved registers.
    rbx: u64,
    rbp: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

/// Captures the current per-CPU saved user state into a local snapshot.
///
/// Must be called immediately after returning from userspace (via
/// `restore_kernel_context`), before any `.await` that could allow
/// another process to overwrite the per-CPU buffers.
fn capture_user_state() -> SavedUserState {
    // SAFETY: We're on the CPU that saved these regs; no concurrent access
    // yet (we haven't yielded to the executor).
    let saved = unsafe { &*SYSCALL_SAVED_REGS.get().get() };
    let user_rsp: u64;
    // SAFETY: GS points to PerCpuState; offset 16 is user_rsp.
    unsafe { core::arch::asm!("mov {}, gs:[16]", out(reg) user_rsp) };

    SavedUserState {
        user_rip: saved.user_rip,
        user_rflags: saved.user_rflags,
        user_rsp,
        rbx: saved.rbx,
        rbp: saved.rbp,
        r12: saved.r12,
        r13: saved.r13,
        r14: saved.r14,
        r15: saved.r15,
    }
}

/// Builds a [`UserRegisters`] for re-entering userspace after a blocking syscall.
///
/// Uses a previously captured [`SavedUserState`] (not the live per-CPU
/// buffers, which may have been overwritten by another process).
fn build_resume_regs(saved: &SavedUserState, return_value: isize) -> UserRegisters {
    UserRegisters {
        rax: return_value as u64,
        rbx: saved.rbx,
        rcx: 0,
        rdx: 0,
        rsi: 0,
        rdi: 0,
        rbp: saved.rbp,
        r8: 0,
        r9: 0,
        r10: 0,
        r11: 0,
        r12: saved.r12,
        r13: saved.r13,
        r14: saved.r14,
        r15: saved.r15,
        rip: saved.user_rip,
        rsp: saved.user_rsp,
        rflags: saved.user_rflags,
    }
}

/// The main async task for running a user process on the executor.
///
/// Spawned once per process. The future is polled by the executor; each poll
/// corresponds to a userspace entry or re-entry after a blocking syscall.
///
/// `address_space` is `None` for the initial userboot process (which runs
/// in the kernel's shared CR3).
#[expect(
    clippy::cast_possible_truncation,
    reason = "koid raw value fits in log output"
)]
pub async fn process_task(
    process: Arc<Process>,
    thread: Arc<Thread>,
    address_space: Option<AddressSpace<PageTableMapper>>,
    entry: u64,
    user_rsp: u64,
) {
    let pid = process.koid().raw();

    // Determine this process's CR3. For userboot (no separate address space),
    // use the current (boot) CR3. For spawned children, use their own PML4.
    let process_cr3 = match address_space {
        Some(ref aspace) => aspace.root_phys(),
        None => crate::arch::x86_64::registers::control::Cr3::read(),
    };

    // Install as current process on this CPU.
    *CURRENT_PROCESS.get().lock() = Some(Arc::clone(&process));
    thread.set_state(ThreadState::Running);

    // Load the process's CR3.
    // SAFETY: process_cr3 points to a valid PML4 with kernel upper half.
    unsafe {
        crate::arch::x86_64::registers::control::Cr3::write(process_cr3);
    }

    // Flush logs before first entry to userspace.
    hadron_log::flush();
    hadron_log::disable_auto_flush();

    // First entry to userspace.
    // enter_userspace_save updates percpu.kernel_rsp and does swapgs internally.
    let mut saved_rsp: u64 = 0;
    // SAFETY: entry/user_rsp point to valid user-mode code/stack.
    // GS is in kernel state. CR3 is loaded with the correct address space.
    unsafe {
        userspace::enter_userspace_save(entry, user_rsp, &mut saved_rsp);
    }

    // Reached here via restore_kernel_context — GS is kernel, IF may be 1.
    // SAFETY: cli is always safe in ring 0.
    unsafe { core::arch::asm!("cli") };
    hadron_log::enable_auto_flush();

    loop {
        // Capture the per-CPU saved register state BEFORE yielding.
        // Another process's syscall on this CPU would overwrite the
        // per-CPU buffers, so we snapshot them into local storage now.
        let user_state = capture_user_state();

        let op = take_blocking_op().expect("returned from userspace without a blocking op");

        match op {
            BlockingOp::Exit(code) => {
                crate::kinfo!("process", "process {} exited with code {}", pid, code);
                process.exit(code as i64);
                thread.exit();

                // Wake any parent waiting for this child.
                wake_child_exit_waiter();
                break;
            }
            BlockingOp::TaskWait {
                pid: child_pid,
                status_ptr,
            } => {
                crate::kdebug!("process", "process {} waiting for child {}", pid, child_pid);

                // Wait for the child process to terminate.
                let child = lookup_process(child_pid).expect("task_wait: child process not found");

                WaitChild::new(Arc::clone(&child)).await;

                let exit_code = child.return_code();

                // Restore CURRENT_PROCESS — another process_task may have
                // overwritten it while we were suspended.
                *CURRENT_PROCESS.get().lock() = Some(Arc::clone(&process));

                // Reload our CR3 — another process may have loaded its own.
                // SAFETY: process_cr3 is valid.
                unsafe {
                    crate::arch::x86_64::registers::control::Cr3::write(process_cr3);
                }

                // Write exit status to user memory if pointer is non-zero.
                if status_ptr != 0 {
                    // SAFETY: status_ptr was validated by the syscall handler.
                    // CR3 is now the parent's address space.
                    unsafe {
                        *(status_ptr as *mut usize) = exit_code as usize;
                    }
                }

                // Re-enter userspace with return value = 0 (success).
                // CR3 was already reloaded above for status_ptr write.
                let regs = build_resume_regs(&user_state, 0);

                hadron_log::flush();
                hadron_log::disable_auto_flush();

                // SAFETY: regs has valid user-mode RIP/RSP. GS is kernel.
                unsafe {
                    userspace::enter_userspace_resume(&regs, &mut saved_rsp);
                }

                // Returned from userspace again.
                // SAFETY: cli is safe in ring 0.
                unsafe { core::arch::asm!("cli") };
                hadron_log::enable_auto_flush();
            }
        }
    }

    // Clean up: close all handles (triggers on_zero_handles for each object).
    process.with_handle_table(|table| table.close_all());
    unregister_process(pid);
    *CURRENT_PROCESS.get().lock() = None;

    // Restore the boot CR3 before dropping the per-process address space.
    // AddressSpace::drop frees the PML4 frame, so we must not be using it.
    if address_space.is_some() {
        let boot_cr3 = crate::vmm::boot_cr3();
        // SAFETY: boot_cr3 is the kernel's root page table, always valid.
        unsafe {
            crate::arch::x86_64::registers::control::Cr3::write(boot_cr3);
        }
    }
}

// ── WaitChild future ────────────────────────────────────────────────

/// A future that resolves when a child process sets the TERMINATED signal.
struct WaitChild {
    /// The child process to wait on.
    child: Arc<Process>,
}

impl WaitChild {
    fn new(child: Arc<Process>) -> Self {
        Self { child }
    }
}

impl Future for WaitChild {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.child.get_signals().contains(Signals::TERMINATED) {
            Poll::Ready(())
        } else {
            // Store our waker so the child's process_task can wake us on exit.
            *CHILD_EXIT_WAKER.get().lock() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
