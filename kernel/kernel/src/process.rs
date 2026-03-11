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
use core::task::{Context, Poll};

use hadron_core::cpu_local::CpuLocal;
use hadron_core::sync::SpinLock;
use hadron_mm::address_space::AddressSpace;
use hadron_syscall::constants::{POLLIN, POLLOUT};

use hadron_objects::channel::{Channel, ChannelError};
use hadron_objects::handle::HandleValue;
use hadron_objects::object::{KernelObject, ObjectType, Signals};
use hadron_objects::observer::WakerDispatch;
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
    /// Blocking channel recv (no handle transfer).
    ChannelRecv {
        /// Channel handle fd.
        fd: usize,
        /// User buffer pointer.
        buf_ptr: usize,
        /// User buffer length.
        buf_len: usize,
    },
    /// Blocking channel recv with handle transfer.
    ChannelRecvFd {
        /// Channel handle fd.
        ch_fd: usize,
        /// User buffer pointer.
        buf_ptr: usize,
        /// User buffer length.
        buf_len: usize,
        /// User pointer to write the received fd.
        fd_out_ptr: usize,
    },
    /// Sleeping until a monotonic deadline (nanoseconds since boot).
    ClockNanosleep {
        /// Absolute deadline in nanoseconds since boot.
        deadline_ns: u64,
    },
    /// Blocking poll: wait until any fd has events or timeout expires.
    EventWaitMany {
        /// User pointer to the PollFd array.
        fds_ptr: usize,
        /// Number of fds.
        nfds: usize,
        /// Timeout in milliseconds (`u64::MAX` = infinite).
        timeout_ms: u64,
    },
    /// Futex wait: block until the futex is woken.
    FutexWait {
        /// User virtual address of the futex word.
        addr: u64,
        /// Expected value (already verified by syscall handler).
        val: u32,
        /// Optional timeout in nanoseconds since boot (`u64::MAX` = no timeout).
        timeout_ns: u64,
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
                // Process::exit() calls signal_update(TERMINATED), which
                // notifies any observer-based wakers registered by WaitChild.
                process.exit(code as i64);
                thread.exit();
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
            BlockingOp::ChannelRecv {
                fd,
                buf_ptr,
                buf_len,
            } => {
                let ch_obj = get_channel_from_fd(&process, fd);

                WaitChannelReadable::new(Arc::clone(&ch_obj)).await;

                // Restore context after suspension.
                *CURRENT_PROCESS.get().lock() = Some(Arc::clone(&process));
                // SAFETY: process_cr3 is valid.
                unsafe {
                    crate::arch::x86_64::registers::control::Cr3::write(process_cr3);
                }

                // Retry the read.
                let result = do_channel_recv(&*ch_obj, buf_ptr, buf_len);
                let regs = build_resume_regs(&user_state, result);

                hadron_log::flush();
                hadron_log::disable_auto_flush();

                // SAFETY: regs has valid user-mode RIP/RSP. GS is kernel.
                unsafe {
                    userspace::enter_userspace_resume(&regs, &mut saved_rsp);
                }
                // SAFETY: cli is safe in ring 0.
                unsafe { core::arch::asm!("cli") };
                hadron_log::enable_auto_flush();
            }
            BlockingOp::ChannelRecvFd {
                ch_fd,
                buf_ptr,
                buf_len,
                fd_out_ptr,
            } => {
                let ch_obj = get_channel_from_fd(&process, ch_fd);

                WaitChannelReadable::new(Arc::clone(&ch_obj)).await;

                // Restore context after suspension.
                *CURRENT_PROCESS.get().lock() = Some(Arc::clone(&process));
                // SAFETY: process_cr3 is valid.
                unsafe {
                    crate::arch::x86_64::registers::control::Cr3::write(process_cr3);
                }

                // Retry the read with handle transfer.
                let result = do_channel_recv_fd(&process, &*ch_obj, buf_ptr, buf_len, fd_out_ptr);
                let regs = build_resume_regs(&user_state, result);

                hadron_log::flush();
                hadron_log::disable_auto_flush();

                // SAFETY: regs has valid user-mode RIP/RSP. GS is kernel.
                unsafe {
                    userspace::enter_userspace_resume(&regs, &mut saved_rsp);
                }
                // SAFETY: cli is safe in ring 0.
                unsafe { core::arch::asm!("cli") };
                hadron_log::enable_auto_flush();
            }
            BlockingOp::ClockNanosleep { deadline_ns } => {
                SleepUntil::new(deadline_ns).await;

                // Restore context after suspension.
                *CURRENT_PROCESS.get().lock() = Some(Arc::clone(&process));
                // SAFETY: process_cr3 is valid.
                unsafe {
                    crate::arch::x86_64::registers::control::Cr3::write(process_cr3);
                }

                // Return 0 (success) to userspace.
                let regs = build_resume_regs(&user_state, 0);

                hadron_log::flush();
                hadron_log::disable_auto_flush();

                // SAFETY: regs has valid user-mode RIP/RSP. GS is kernel.
                unsafe {
                    userspace::enter_userspace_resume(&regs, &mut saved_rsp);
                }
                // SAFETY: cli is safe in ring 0.
                unsafe { core::arch::asm!("cli") };
                hadron_log::enable_auto_flush();
            }
            BlockingOp::FutexWait {
                addr,
                val: _,
                timeout_ns,
            } => {
                WaitFutex::new(pid, addr, timeout_ns).await;

                // Restore context after suspension.
                *CURRENT_PROCESS.get().lock() = Some(Arc::clone(&process));
                // SAFETY: process_cr3 is valid.
                unsafe {
                    crate::arch::x86_64::registers::control::Cr3::write(process_cr3);
                }

                // Return 0 to userspace (woken successfully or timed out).
                let regs = build_resume_regs(&user_state, 0);

                hadron_log::flush();
                hadron_log::disable_auto_flush();

                // SAFETY: regs has valid user-mode RIP/RSP. GS is kernel.
                unsafe {
                    userspace::enter_userspace_resume(&regs, &mut saved_rsp);
                }
                // SAFETY: cli is safe in ring 0.
                unsafe { core::arch::asm!("cli") };
                hadron_log::enable_auto_flush();
            }
            BlockingOp::EventWaitMany {
                fds_ptr,
                nfds,
                timeout_ms,
            } => {
                // Collect kernel objects for each fd.
                let entries: alloc::vec::Vec<PollEntry> = process.with_handle_table(|table| {
                    // SAFETY: fds_ptr was validated by the syscall handler.
                    let fds = unsafe {
                        core::slice::from_raw_parts(
                            fds_ptr as *const hadron_syscall::types::PollFd,
                            nfds,
                        )
                    };
                    fds.iter()
                        .filter_map(|pfd| {
                            let hv = HandleValue::from_raw(pfd.fd);
                            table.get(hv).ok().map(|entry| PollEntry {
                                object: Arc::clone(entry.object()),
                                events: pfd.events,
                            })
                        })
                        .collect()
                });

                let timeout_ns = if timeout_ms == u64::MAX {
                    u64::MAX
                } else {
                    crate::time::nanos_since_boot().saturating_add(timeout_ms as u64 * 1_000_000)
                };

                WaitAnyReady::new(entries, timeout_ns).await;

                // Restore context after suspension.
                *CURRENT_PROCESS.get().lock() = Some(Arc::clone(&process));
                // SAFETY: process_cr3 is valid.
                unsafe {
                    crate::arch::x86_64::registers::control::Cr3::write(process_cr3);
                }

                // Re-poll all fds and write back revents, then return count.
                let result = crate::syscall::event::sys_event_wait_many(fds_ptr, nfds, 0);
                let regs = build_resume_regs(&user_state, result);

                hadron_log::flush();
                hadron_log::disable_auto_flush();

                // SAFETY: regs has valid user-mode RIP/RSP. GS is kernel.
                unsafe {
                    userspace::enter_userspace_resume(&regs, &mut saved_rsp);
                }
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
///
/// Uses observer-based waking: registers a [`WakerDispatch`] on the child
/// process for `TERMINATED`. When the child exits and signals TERMINATED,
/// the observer fires and wakes this future. Works correctly across CPUs.
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
            // Register an observer-based waker on the child process.
            // The observer is one-shot: it is removed when it fires.
            let dispatch = Arc::new(WakerDispatch::new(cx.waker().clone()));
            self.child.add_observer(dispatch, 0, Signals::TERMINATED);
            // Re-check after registration to avoid missed wake-up.
            if self.child.get_signals().contains(Signals::TERMINATED) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }
}

// ── WaitChannelReadable future ──────────────────────────────────────

/// A future that resolves when a kernel object has the READABLE signal set.
///
/// Uses observer-based waking: registers a [`WakerDispatch`] on the object
/// for `READABLE`. When the object becomes readable (e.g. channel write),
/// the observer fires and wakes this future.
struct WaitChannelReadable {
    /// The channel object to wait on.
    object: Arc<dyn KernelObject>,
}

impl WaitChannelReadable {
    fn new(object: Arc<dyn KernelObject>) -> Self {
        Self { object }
    }
}

impl Future for WaitChannelReadable {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.object.get_signals().contains(Signals::READABLE) {
            Poll::Ready(())
        } else {
            let dispatch = Arc::new(WakerDispatch::new(cx.waker().clone()));
            self.object.add_observer(dispatch, 0, Signals::READABLE);
            // Re-check after registration to avoid missed wake-up.
            if self.object.get_signals().contains(Signals::READABLE) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }
}

// ── SleepUntil future ──────────────────────────────────────────────

/// A future that resolves when the monotonic clock reaches a deadline.
struct SleepUntil {
    /// Absolute deadline in nanoseconds since boot.
    deadline_ns: u64,
    /// Whether the waker has been registered in the sleep queue.
    registered: bool,
}

impl SleepUntil {
    fn new(deadline_ns: u64) -> Self {
        Self {
            deadline_ns,
            registered: false,
        }
    }
}

impl Future for SleepUntil {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if crate::time::nanos_since_boot() >= self.deadline_ns {
            Poll::Ready(())
        } else {
            if !self.registered {
                self.registered = true;
            }
            // Re-register on every poll in case the waker changed.
            hadron_sched::timer::register_sleep_waker(self.deadline_ns, cx.waker().clone());
            Poll::Pending
        }
    }
}

// ── WaitFutex future ───────────────────────────────────────────────

/// A future that resolves when a futex is woken or a timeout expires.
struct WaitFutex {
    /// Process koid (part of the futex key).
    koid: u64,
    /// Virtual address of the futex word.
    addr: u64,
    /// Timeout deadline in nanoseconds since boot, or `u64::MAX` for none.
    timeout_ns: u64,
    /// Whether the waker has been registered in the futex table.
    registered: bool,
}

impl WaitFutex {
    fn new(koid: u64, addr: u64, timeout_ns: u64) -> Self {
        Self {
            koid,
            addr,
            timeout_ns,
            registered: false,
        }
    }
}

impl Future for WaitFutex {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // Check timeout first.
        if self.timeout_ns != u64::MAX && crate::time::nanos_since_boot() >= self.timeout_ns {
            return Poll::Ready(());
        }

        if !self.registered {
            self.registered = true;
            crate::futex::futex_wait(self.koid, self.addr, cx.waker().clone());

            // Register a timeout waker if needed.
            if self.timeout_ns != u64::MAX {
                hadron_sched::timer::register_sleep_waker(self.timeout_ns, cx.waker().clone());
            }

            Poll::Pending
        } else {
            // We were woken (either by futex_wake or timeout).
            Poll::Ready(())
        }
    }
}

// ── WaitAnyReady future ────────────────────────────────────────────

/// Per-fd info for the `WaitAnyReady` future.
struct PollEntry {
    /// The kernel object being polled.
    object: Arc<dyn KernelObject>,
    /// Requested event mask (POLLIN/POLLOUT bits).
    events: u16,
}

/// A future that resolves when any polled fd has matching signals.
///
/// Uses observer-based waking: registers `WakerDispatch` on each object
/// for the requested signal types. When any object's signals change, the
/// future re-polls all objects. Level-triggered semantics (like poll/select).
struct WaitAnyReady {
    /// Objects to poll.
    entries: alloc::vec::Vec<PollEntry>,
    /// Timeout deadline in nanoseconds since boot, or `u64::MAX` for none.
    timeout_ns: u64,
}

impl WaitAnyReady {
    fn new(entries: alloc::vec::Vec<PollEntry>, timeout_ns: u64) -> Self {
        Self {
            entries,
            timeout_ns,
        }
    }
}

impl Future for WaitAnyReady {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // Check if any fd is already ready.
        for entry in &self.entries {
            let sig = entry.object.get_signals();
            let has_readable = entry.events & POLLIN != 0 && sig.contains(Signals::READABLE);
            let has_writable = entry.events & POLLOUT != 0 && sig.contains(Signals::WRITABLE);
            let has_hangup = sig.contains(Signals::PEER_CLOSED);
            if has_readable || has_writable || has_hangup {
                return Poll::Ready(());
            }
        }

        // Check timeout.
        if self.timeout_ns != u64::MAX && crate::time::nanos_since_boot() >= self.timeout_ns {
            return Poll::Ready(());
        }

        // Not ready yet — register observers on each object.
        for entry in &self.entries {
            let mut mask = Signals::PEER_CLOSED;
            if entry.events & POLLIN != 0 {
                mask |= Signals::READABLE;
            }
            if entry.events & POLLOUT != 0 {
                mask |= Signals::WRITABLE;
            }
            let dispatch = Arc::new(WakerDispatch::new(cx.waker().clone()));
            entry.object.add_observer(dispatch, 0, mask);
        }

        // Register timeout waker if needed.
        if self.timeout_ns != u64::MAX {
            hadron_sched::timer::register_sleep_waker(self.timeout_ns, cx.waker().clone());
        }

        Poll::Pending
    }
}

// ── Channel recv helpers ────────────────────────────────────────────

/// Looks up a channel object from the process handle table by fd.
///
/// Returns the `Arc<dyn KernelObject>` for the channel.
///
/// # Panics
///
/// Panics if the fd is invalid or does not refer to a Channel. The syscall
/// handler validated the fd before setting the blocking op.
fn get_channel_from_fd(process: &Arc<Process>, fd: usize) -> Arc<dyn KernelObject> {
    let hv = HandleValue::from_raw(fd as u32);
    process.with_handle_table(|table| {
        let entry = table
            .get(hv)
            .expect("channel_recv: invalid fd after blocking op");
        assert_eq!(
            entry.object().object_type(),
            ObjectType::Channel,
            "channel_recv: fd is not a channel"
        );
        Arc::clone(entry.object())
    })
}

/// Downcasts a `dyn KernelObject` to `&Channel`.
fn as_channel(obj: &dyn KernelObject) -> &Channel {
    obj.as_any()
        .downcast_ref::<Channel>()
        .expect("channel_recv: object is not a channel")
}

/// Performs the actual channel read, copying data to the user buffer.
///
/// Returns the message length on success, or a negative errno.
#[expect(
    clippy::cast_possible_truncation,
    reason = "message lengths fit in isize"
)]
fn do_channel_recv(obj: &dyn KernelObject, buf_ptr: usize, buf_len: usize) -> isize {
    let channel = as_channel(obj);
    match channel.read() {
        Ok(msg) => {
            let copy_len = msg.data.len().min(buf_len);
            // SAFETY: buf_ptr was validated by the syscall handler before
            // the blocking op was set. CR3 has been restored.
            unsafe {
                core::ptr::copy_nonoverlapping(msg.data.as_ptr(), buf_ptr as *mut u8, copy_len);
            }
            msg.data.len() as isize
        }
        Err(ChannelError::ShouldWait) => -hadron_syscall::EAGAIN,
        Err(ChannelError::PeerClosed) => -hadron_syscall::EPIPE,
        Err(_) => -hadron_syscall::EIO,
    }
}

/// Performs channel read with handle transfer, installing received handles.
#[expect(
    clippy::cast_possible_truncation,
    reason = "message lengths fit in isize"
)]
fn do_channel_recv_fd(
    process: &Arc<Process>,
    obj: &dyn KernelObject,
    buf_ptr: usize,
    buf_len: usize,
    fd_out_ptr: usize,
) -> isize {
    let channel = as_channel(obj);
    match channel.read() {
        Ok(msg) => {
            let copy_len = msg.data.len().min(buf_len);
            // SAFETY: buf_ptr was validated before the blocking op. CR3 restored.
            unsafe {
                core::ptr::copy_nonoverlapping(msg.data.as_ptr(), buf_ptr as *mut u8, copy_len);
            }

            // Install transferred handles into the receiver's table.
            let received_fd = if let Some(handle) = msg.handles.into_iter().next() {
                process.with_handle_table(|table| match table.insert(handle) {
                    Ok(hv) => hv.raw() as usize,
                    Err(_) => usize::MAX,
                })
            } else {
                usize::MAX // No handle attached
            };

            // SAFETY: fd_out_ptr was validated before the blocking op. CR3 restored.
            unsafe {
                *(fd_out_ptr as *mut usize) = received_fd;
            }
            msg.data.len() as isize
        }
        Err(ChannelError::ShouldWait) => -hadron_syscall::EAGAIN,
        Err(ChannelError::PeerClosed) => -hadron_syscall::EPIPE,
        Err(_) => -hadron_syscall::EIO,
    }
}
