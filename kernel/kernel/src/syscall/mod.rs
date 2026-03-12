//! Kernel syscall dispatch and handler modules.
//!
//! The architecture-specific entry stub (`arch/x86_64/syscall.rs`) calls
//! [`dispatch`] after remapping registers. Each handler module implements
//! a group of related syscalls.

extern crate alloc;

pub mod channel;
pub mod event;
pub mod event_pair;
pub mod fifo;
pub mod handle;
#[cfg(hadron_iommu)]
pub mod iommu;
pub mod memory;
pub mod net;
pub mod port;
pub mod system;
pub mod task;
pub mod timer;
pub mod validate;
pub mod vnode;

use hadron_objects::handle::HandleTable;
use hadron_syscall::*;

/// Execute a closure with the current process's handle table.
///
/// Uses the per-process handle table from CURRENT_PROCESS.
/// Falls back to a stub that panics if no process is active.
pub fn with_handle_table<R>(f: impl FnOnce(&mut HandleTable) -> R) -> R {
    crate::process::with_current_process(|proc| proc.with_handle_table(f))
        .expect("with_handle_table called with no active process")
}

/// Syscall dispatch — routes a syscall number to the appropriate handler.
///
/// Called from `arch::x86_64::syscall::syscall_dispatch()` with arguments
/// already remapped from the Linux syscall ABI.
pub fn dispatch(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize {
    match nr {
        // ── Task ─────────────────────────────────────────────────
        SYS_TASK_EXIT => task::sys_task_exit(a0),
        SYS_TASK_SPAWN => task::sys_task_spawn(a0, a1),
        SYS_TASK_WAIT => task::sys_task_wait(a0, a1, a2),
        SYS_TASK_INFO => task::sys_task_info(),
        SYS_TASK_KILL => task::sys_task_kill(a0, a1),
        SYS_TASK_SIGACTION => task::sys_task_sigaction(a0, a1, a2, a3),
        SYS_TASK_SETPGID => task::sys_task_setpgid(a0, a1),
        SYS_TASK_GETPGID => task::sys_task_getpgid(a0),

        // ── Handle ───────────────────────────────────────────────
        SYS_HANDLE_CLOSE => handle::sys_handle_close(a0),
        SYS_HANDLE_DUP => handle::sys_handle_dup(a0, a1),
        SYS_HANDLE_DUP_LOWEST => handle::sys_handle_dup_lowest(a0),
        SYS_HANDLE_PIPE => handle::sys_handle_pipe(a0),
        SYS_HANDLE_TCSETPGRP => handle::sys_handle_tcsetpgrp(a0, a1),
        SYS_HANDLE_TCGETPGRP => handle::sys_handle_tcgetpgrp(a0),

        // ── Channel / IPC ────────────────────────────────────────
        SYS_CHANNEL_CREATE => channel::sys_channel_create(a0),
        SYS_CHANNEL_SEND => channel::sys_channel_send(a0, a1, a2),
        SYS_CHANNEL_RECV => channel::sys_channel_recv(a0, a1, a2),
        SYS_CHANNEL_ACCEPT => -ENOSYS, // Phase 5+: service namespace
        SYS_CHANNEL_SEND_FD => channel::sys_channel_send_fd(a0, a1, a2, a3),
        SYS_CHANNEL_RECV_FD => channel::sys_channel_recv_fd(a0, a1, a2, a3),
        SYS_EVENT_PAIR_CREATE => event_pair::sys_event_pair_create(a0),
        SYS_EVENT_PAIR_SIGNAL_PEER => event_pair::sys_event_pair_signal_peer(a0, a1, a2),
        SYS_FIFO_CREATE => fifo::sys_fifo_create(a0, a1, a2),
        SYS_FIFO_WRITE => fifo::sys_fifo_write(a0, a1, a2),
        SYS_FIFO_READ => fifo::sys_fifo_read(a0, a1, a2),

        // ── Vnode ───────────────────────────────────────────────
        SYS_VNODE_OPEN => vnode::sys_vnode_open(a0, a1, a2),
        SYS_VNODE_READ => vnode::sys_vnode_read(a0, a1, a2),
        SYS_VNODE_WRITE => vnode::sys_vnode_write(a0, a1, a2),
        SYS_VNODE_STAT => vnode::sys_vnode_stat(a0, a1),
        SYS_VNODE_READDIR => vnode::sys_vnode_readdir(a0, a1, a2),
        SYS_VNODE_UNLINK => vnode::sys_vnode_unlink(a0, a1),
        SYS_VNODE_SEEK => vnode::sys_vnode_seek(a0, a1, a2),
        SYS_VNODE_MKDIR => vnode::sys_vnode_mkdir(a0, a1, a2),
        SYS_VNODE_RENAME => vnode::sys_vnode_rename(a0, a1, a2, a3),
        SYS_VNODE_SYMLINK => vnode::sys_vnode_symlink(a0, a1, a2, a3),
        SYS_VNODE_LINK => vnode::sys_vnode_link(a0, a1, a2, a3),
        SYS_VNODE_READLINK => vnode::sys_vnode_readlink(a0, a1, a2),
        SYS_VNODE_TRUNCATE => vnode::sys_vnode_truncate(a0, a1),
        SYS_VNODE_FSTATAT => vnode::sys_vnode_fstatat(a0, a1, a2, a3),

        // ── Memory ───────────────────────────────────────────────
        SYS_MEM_MAP => memory::sys_mem_map(a0, a1, a2, a3, a4),
        SYS_MEM_UNMAP => memory::sys_mem_unmap(a0, a1),
        SYS_MEM_BRK => memory::sys_mem_brk(a0),
        SYS_MEM_CREATE_SHARED => memory::sys_mem_create_shared(a0),
        SYS_MEM_MAP_SHARED => memory::sys_mem_map_shared(a0, a1, a2),

        // ── Event / Timer ────────────────────────────────────────
        SYS_EVENT_CREATE => event::sys_event_create(),
        SYS_EVENT_SIGNAL => event::sys_event_signal(a0, a1, a2),
        SYS_EVENT_WAIT => event::sys_event_wait(a0, a1),
        SYS_EVENT_WAIT_MANY => event::sys_event_wait_many(a0, a1, a2),
        SYS_CLOCK_GETTIME => event::sys_clock_gettime(a0, a1),
        SYS_CLOCK_NANOSLEEP => event::sys_clock_nanosleep(a0, a1, a2, a3),
        SYS_FUTEX => event::sys_futex(a0, a1, a2, a3),
        SYS_PORT_CREATE => port::sys_port_create(),
        SYS_PORT_WAIT => port::sys_port_wait(a0, a1),
        SYS_PORT_QUEUE => port::sys_port_queue(a0, a1, a2),
        SYS_TIMER_CREATE => timer::sys_timer_create(),
        SYS_TIMER_SET => timer::sys_timer_set(a0, a1, a2),
        SYS_TIMER_CANCEL => timer::sys_timer_cancel(a0),
        SYS_OBJECT_WAIT_ASYNC => port::sys_object_wait_async(a0, a1, a2, a3),

        // ── Network ──────────────────────────────────────────────
        SYS_NET_SOCKET => net::sys_net_socket(a0, a1, a2),
        SYS_NET_BIND => net::sys_net_bind(a0, a1, a2),
        SYS_NET_LISTEN => net::sys_net_listen(a0, a1),
        SYS_NET_ACCEPT => net::sys_net_accept(a0, a1, a2),
        SYS_NET_CONNECT => net::sys_net_connect(a0, a1, a2),
        SYS_NET_SENDMSG => net::sys_net_sendmsg(a0, a1, a2),
        SYS_NET_RECVMSG => net::sys_net_recvmsg(a0, a1, a2),
        SYS_NET_SHUTDOWN => net::sys_net_shutdown(a0, a1),

        // ── Device / IOMMU ──────────────────────────────────────
        #[cfg(hadron_iommu)]
        SYS_BTI_CREATE => iommu::sys_bti_create(a0, a1),
        #[cfg(hadron_iommu)]
        SYS_BTI_PIN => iommu::sys_bti_pin(a0, a1, a2, a3, a4),
        #[cfg(hadron_iommu)]
        SYS_BTI_RELEASE_QUARANTINE => iommu::sys_bti_release_quarantine(a0),
        #[cfg(hadron_iommu)]
        SYS_PMT_UNPIN => iommu::sys_pmt_unpin(a0),

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
