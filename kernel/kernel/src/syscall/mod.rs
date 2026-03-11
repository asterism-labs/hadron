//! Kernel syscall dispatch and handler modules.
//!
//! The architecture-specific entry stub (`arch/x86_64/syscall.rs`) calls
//! [`dispatch`] after remapping registers. Each handler module implements
//! a group of related syscalls.

extern crate alloc;

pub mod channel;
pub mod handle;
pub mod system;
pub mod validate;

use hadron_core::sync::SpinLock;
use hadron_objects::handle::HandleTable;
use hadron_syscall::*;

/// Global handle table for Phase 2a (single process, shared CR3).
///
/// Phase 2b replaces this with per-process handle tables accessed via
/// `CURRENT_CONTEXT`.
static GLOBAL_HANDLE_TABLE: SpinLock<HandleTable> = SpinLock::new(HandleTable::new());

/// Execute a closure with access to the current process's handle table.
///
/// Phase 2a: uses the global handle table. Phase 2b: uses the process
/// from `CURRENT_CONTEXT`.
pub fn with_handle_table<R>(f: impl FnOnce(&mut HandleTable) -> R) -> R {
    f(&mut GLOBAL_HANDLE_TABLE.lock())
}

/// Syscall dispatch — routes a syscall number to the appropriate handler.
///
/// Called from `arch::x86_64::syscall::syscall_dispatch()` with arguments
/// already remapped from the Linux syscall ABI.
pub fn dispatch(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize, _a4: usize) -> isize {
    match nr {
        // ── Task ─────────────────────────────────────────────────
        SYS_TASK_EXIT => sys_task_exit(a0),
        SYS_TASK_INFO => sys_task_info(),

        // ── Handle ───────────────────────────────────────────────
        SYS_HANDLE_CLOSE => handle::sys_handle_close(a0),
        SYS_HANDLE_DUP => handle::sys_handle_dup(a0, a1),
        SYS_HANDLE_DUP_LOWEST => handle::sys_handle_dup_lowest(a0),
        SYS_HANDLE_PIPE => handle::sys_handle_pipe(a0),

        // ── Channel ──────────────────────────────────────────────
        SYS_CHANNEL_CREATE => channel::sys_channel_create(a0),
        SYS_CHANNEL_SEND => channel::sys_channel_send(a0, a1, a2),
        SYS_CHANNEL_RECV => channel::sys_channel_recv(a0, a1, a2),
        SYS_CHANNEL_SEND_FD => channel::sys_channel_send_fd(a0, a1, a2, a3),
        SYS_CHANNEL_RECV_FD => channel::sys_channel_recv_fd(a0, a1, a2, a3),

        // ── System ───────────────────────────────────────────────
        SYS_DEBUG_LOG => system::sys_debug_log(a0, a1),
        SYS_QUERY => system::sys_query(a0, a1, a2, a3),

        // ── Unimplemented ────────────────────────────────────────
        _ => {
            crate::kwarn!("syscall", "unimplemented syscall 0x{:02x}", nr);
            -ENOSYS
        }
    }
}

/// `SYS_TASK_EXIT(code)` — terminate the current task.
///
/// In Phase 2a (no process lifecycle yet), this logs the exit code and
/// halts the CPU.
fn sys_task_exit(code: usize) -> isize {
    hadron_log::enable_auto_flush();
    crate::kinfo!("syscall", "task exited with code {}", code);
    hadron_log::flush();

    loop {
        // SAFETY: HLT is always safe in ring 0.
        unsafe { core::arch::asm!("hlt") };
    }
}

/// `SYS_TASK_INFO()` — return current process koid (PID).
///
/// Phase 2a stub: returns 1 (userboot is always PID 1).
fn sys_task_info() -> isize {
    1
}
