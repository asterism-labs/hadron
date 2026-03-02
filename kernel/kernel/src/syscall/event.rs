//! Event syscall handlers: `event_wait_many` (poll) and `futex`.

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::fs::Inode;
use crate::id::Fd;
use crate::proc::ProcessTable;
use crate::syscall::userptr::UserSlice;
use crate::syscall::{EFAULT, EINVAL, POLLNVAL};
use hadron_syscall::PollFd;

/// `sys_event_wait_many` — poll multiple file descriptors for readiness.
///
/// Scans each fd for readiness (POLLIN/POLLOUT), fills in `revents`, and
/// returns the count of fds with non-zero `revents`.
///
/// `timeout_ms`: 0 = non-blocking, `usize::MAX` = infinite (blocks via trap),
/// other values = timeout in ms (blocks via trap).
///
/// For the initial implementation, only non-blocking (timeout_ms == 0) and
/// single-shot polling is supported. Blocking poll is handled by retrying
/// in userspace or via the trap mechanism in a future iteration.
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning negated errno or small count as isize"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "fd fits in u32; count fits in isize"
)]
pub(super) fn sys_event_wait_many(fds_ptr: usize, nfds: usize, timeout_ms: usize) -> isize {
    if nfds == 0 {
        if timeout_ms > 0 && timeout_ms != usize::MAX {
            // Pure sleep — delegate to nanosleep equivalent.
            // For now, return 0 (no fds ready).
            return 0;
        }
        return 0;
    }

    // Validate user pointer covers the entire PollFd array.
    let total_size = nfds.saturating_mul(core::mem::size_of::<PollFd>());
    if UserSlice::new(fds_ptr, total_size).is_err() {
        return -EFAULT;
    }

    // Read the PollFd array from user memory.
    // SAFETY: UserSlice validated the pointer is in user space and properly sized.
    let poll_fds = unsafe { core::slice::from_raw_parts_mut(fds_ptr as *mut PollFd, nfds) };

    // Phase 1: clone all inodes while holding fd_table.
    //
    // poll_readiness() on unix sockets acquires unix_socket (SpinLock, level 3).
    // fd_table is level 4, so calling poll_readiness() while fd_table is locked
    // would violate lock ordering.  We collect Arc clones here — cheap and safe
    // inside the lock — then call poll_readiness() after releasing it.
    // None means the fd was not found (→ POLLNVAL).
    let inodes: Vec<Option<Arc<dyn Inode>>> = ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        poll_fds
            .iter()
            .map(|pfd| fd_table.get(Fd::new(pfd.fd)).map(|f| f.inode.clone()))
            .collect()
    });
    // fd_table and CURRENT_PROCESS are released above.

    // Phase 2: call poll_readiness() outside all locks.
    let mut ready_count: isize = 0;
    for (pfd, inode_opt) in poll_fds.iter_mut().zip(inodes.iter()) {
        pfd.revents = 0;
        match inode_opt {
            None => {
                pfd.revents = POLLNVAL;
                ready_count += 1;
            }
            Some(inode) => {
                let readiness = inode.poll_readiness(None);
                pfd.revents = readiness
                    & (pfd.events
                        | hadron_syscall::POLLERR
                        | hadron_syscall::POLLHUP
                        | hadron_syscall::POLLNVAL);
                if pfd.revents != 0 {
                    ready_count += 1;
                }
            }
        }
    }

    // If nothing is ready and timeout > 0, we'd need to block.
    // For now, return the count (0 means nothing ready in non-blocking mode).
    if ready_count == 0 && timeout_ms > 0 {
        // TODO: implement blocking poll via trap mechanism.
        // For now, return 0 to indicate nothing ready (non-blocking behavior).
        return 0;
    }

    ready_count
}

/// `sys_futex` — fast userspace mutex operations.
///
/// - `FUTEX_WAIT` (op=0): If `*(u32*)addr == val`, sleep until woken.
///   Uses the trap mechanism to longjmp back to `process_task` for async await.
/// - `FUTEX_WAKE` (op=1): Wake up to `val` waiters sleeping on `addr`.
///   Returns the number of waiters actually woken.
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning negated errno or small count as isize"
)]
pub(super) fn sys_futex(addr: usize, op: usize, val: usize, _timeout_ms: usize) -> isize {
    use hadron_syscall::{FUTEX_WAIT, FUTEX_WAKE};

    match op {
        FUTEX_WAIT => {
            // Validate the user address.
            if UserSlice::new(addr, core::mem::size_of::<u32>()).is_err() {
                return -EFAULT;
            }

            #[expect(clippy::cast_possible_truncation, reason = "futex values are u32")]
            let expected = val as u32;

            // Longjmp to process_task for async futex wait.
            trap_futex(addr, expected);
        }
        FUTEX_WAKE => {
            let woken = crate::ipc::futex::futex_wake(addr, val);
            #[expect(clippy::cast_possible_wrap, reason = "woken count fits in isize")]
            {
                woken as isize
            }
        }
        _ => -EINVAL,
    }
}

/// Trigger a `TRAP_FUTEX` longjmp back to `process_task`.
///
/// Sets the futex address and expected value, restores kernel CR3 and
/// GS bases, then calls `restore_kernel_context` — never returns.
fn trap_futex(addr: usize, expected: u32) -> ! {
    use crate::arch::x86_64::registers::control::Cr3;
    use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
    use crate::arch::x86_64::userspace::restore_kernel_context;

    let kernel_cr3 = crate::proc::TrapContext::kernel_cr3();

    // SAFETY: Restoring kernel CR3 and GS bases is the standard pattern
    // for returning from userspace context to kernel context.
    unsafe {
        Cr3::write(kernel_cr3);
        let percpu = IA32_GS_BASE.read();
        IA32_KERNEL_GS_BASE.write(percpu);
    }

    crate::proc::FutexState::set_params(addr as u64, u64::from(expected));
    crate::proc::TrapContext::set_trap_reason(crate::proc::TrapReason::Futex);

    let saved_rsp = crate::proc::TrapContext::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save,
    // still valid on the executor stack.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}
