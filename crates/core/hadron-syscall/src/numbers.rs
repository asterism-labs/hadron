//! Syscall number constants.
//!
//! Numbers are grouped by subsystem with a fixed base per group. The kernel
//! dispatch table matches on these constants to route to the correct handler.

// ── Task (0x00) ──────────────────────────────────────────────────────

/// Terminate the current process.
pub const SYS_TASK_EXIT: usize = 0x00;
/// Spawn a new process from an ELF binary.
pub const SYS_TASK_SPAWN: usize = 0x01;
/// Wait for a child process to exit.
pub const SYS_TASK_WAIT: usize = 0x02;
/// Send a signal to a process.
pub const SYS_TASK_KILL: usize = 0x03;
/// Clone the current thread (reserved).
pub const SYS_TASK_CLONE: usize = 0x04;
/// Query current process/thread information.
pub const SYS_TASK_INFO: usize = 0x05;
/// Register a signal handler.
pub const SYS_TASK_SIGACTION: usize = 0x06;
/// Return from a signal handler.
pub const SYS_TASK_SIGRETURN: usize = 0x07;
/// Set process group ID.
pub const SYS_TASK_SETPGID: usize = 0x08;
/// Get process group ID.
pub const SYS_TASK_GETPGID: usize = 0x09;
/// Get parent process ID.
pub const SYS_TASK_GETPPID: usize = 0x0A;
/// Get current working directory.
pub const SYS_TASK_GETCWD: usize = 0x0B;
/// Change current working directory.
pub const SYS_TASK_CHDIR: usize = 0x0C;
/// Create a new session.
pub const SYS_TASK_SETSID: usize = 0x0D;
/// Set signal mask.
pub const SYS_TASK_SIGPROCMASK: usize = 0x0E;
/// Replace current process image.
pub const SYS_TASK_EXECVE: usize = 0x0F;

// ── Handle (0x10) ────────────────────────────────────────────────────

/// Close a handle (file descriptor).
pub const SYS_HANDLE_CLOSE: usize = 0x10;
/// Duplicate a handle to a specific slot.
pub const SYS_HANDLE_DUP: usize = 0x11;
/// Duplicate a handle to the lowest available slot.
pub const SYS_HANDLE_DUP_LOWEST: usize = 0x12;
/// Create a unidirectional pipe (socket pair).
pub const SYS_HANDLE_PIPE: usize = 0x13;
/// Set terminal foreground process group.
pub const SYS_HANDLE_TCSETPGRP: usize = 0x14;
/// Get terminal foreground process group.
pub const SYS_HANDLE_TCGETPGRP: usize = 0x15;
/// Device I/O control.
pub const SYS_HANDLE_IOCTL: usize = 0x16;
/// File descriptor control.
pub const SYS_HANDLE_FCNTL: usize = 0x17;
/// Create a pipe with flags.
pub const SYS_HANDLE_PIPE2: usize = 0x18;

// ── Channel / IPC (0x20) ─────────────────────────────────────────────

/// Create a bidirectional channel pair.
pub const SYS_CHANNEL_CREATE: usize = 0x20;
/// Send a message on a channel endpoint.
pub const SYS_CHANNEL_SEND: usize = 0x21;
/// Receive a message from a channel endpoint.
pub const SYS_CHANNEL_RECV: usize = 0x22;
/// Accept a pending connection on a listener.
pub const SYS_CHANNEL_ACCEPT: usize = 0x23;
/// Send a message with an attached handle.
pub const SYS_CHANNEL_SEND_FD: usize = 0x24;
/// Receive a message with an attached handle.
pub const SYS_CHANNEL_RECV_FD: usize = 0x25;
/// Create an event pair (two linked event endpoints).
pub const SYS_EVENT_PAIR_CREATE: usize = 0x26;
/// Signal the peer of an event pair.
pub const SYS_EVENT_PAIR_SIGNAL_PEER: usize = 0x27;
/// Create a FIFO pair.
pub const SYS_FIFO_CREATE: usize = 0x28;
/// Write elements to a FIFO.
pub const SYS_FIFO_WRITE: usize = 0x29;
/// Read elements from a FIFO.
pub const SYS_FIFO_READ: usize = 0x2A;

// ── Vnode (0x30) ─────────────────────────────────────────────────────

/// Open a file or directory.
pub const SYS_VNODE_OPEN: usize = 0x30;
/// Read from a file descriptor.
pub const SYS_VNODE_READ: usize = 0x31;
/// Write to a file descriptor.
pub const SYS_VNODE_WRITE: usize = 0x32;
/// Get file status.
pub const SYS_VNODE_STAT: usize = 0x33;
/// Read directory entries.
pub const SYS_VNODE_READDIR: usize = 0x34;
/// Remove a directory entry.
pub const SYS_VNODE_UNLINK: usize = 0x35;
/// Seek within a file.
pub const SYS_VNODE_SEEK: usize = 0x36;
/// Create a directory.
pub const SYS_VNODE_MKDIR: usize = 0x37;
/// Rename a file or directory.
pub const SYS_VNODE_RENAME: usize = 0x38;
/// Create a symbolic link.
pub const SYS_VNODE_SYMLINK: usize = 0x39;
/// Create a hard link.
pub const SYS_VNODE_LINK: usize = 0x3A;
/// Read a symbolic link target.
pub const SYS_VNODE_READLINK: usize = 0x3B;
/// Truncate a file.
pub const SYS_VNODE_TRUNCATE: usize = 0x3C;
/// Get file status relative to directory.
pub const SYS_VNODE_FSTATAT: usize = 0x3D;
/// Mount a filesystem at a path prefix.
pub const SYS_VFS_MOUNT: usize = 0x3E;
/// Unmount a filesystem from a path prefix.
pub const SYS_VFS_UNMOUNT: usize = 0x3F;

// ── Memory (0x40) ────────────────────────────────────────────────────

/// Map memory into the process address space.
pub const SYS_MEM_MAP: usize = 0x40;
/// Unmap a memory region.
pub const SYS_MEM_UNMAP: usize = 0x41;
/// Adjust the program break.
pub const SYS_MEM_BRK: usize = 0x42;
/// Create a shared memory object (VMO).
pub const SYS_MEM_CREATE_SHARED: usize = 0x43;
/// Map a shared memory object.
pub const SYS_MEM_MAP_SHARED: usize = 0x44;
/// Change memory protection flags.
pub const SYS_MEM_PROTECT: usize = 0x45;
/// Query the size of a VMO.
pub const SYS_VMO_GET_SIZE: usize = 0x46;

// ── Event (0x50) ─────────────────────────────────────────────────────

/// Create an event object.
pub const SYS_EVENT_CREATE: usize = 0x50;
/// Signal an event object.
pub const SYS_EVENT_SIGNAL: usize = 0x51;
/// Wait on a single event.
pub const SYS_EVENT_WAIT: usize = 0x52;
/// Poll multiple file descriptors for events.
pub const SYS_EVENT_WAIT_MANY: usize = 0x53;
/// Get the current time.
pub const SYS_CLOCK_GETTIME: usize = 0x54;
/// Sleep for a specified duration.
pub const SYS_CLOCK_NANOSLEEP: usize = 0x55;
/// Futex operations (wait/wake).
pub const SYS_FUTEX: usize = 0x56;
/// Create a port (async event aggregator).
pub const SYS_PORT_CREATE: usize = 0x57;
/// Wait for a packet on a port.
pub const SYS_PORT_WAIT: usize = 0x58;
/// Queue a user packet on a port.
pub const SYS_PORT_QUEUE: usize = 0x59;
/// Create a timer object.
pub const SYS_TIMER_CREATE: usize = 0x5A;
/// Set a timer deadline.
pub const SYS_TIMER_SET: usize = 0x5B;
/// Cancel a pending timer.
pub const SYS_TIMER_CANCEL: usize = 0x5C;
/// Register an async wait on an object's signals via a port.
pub const SYS_OBJECT_WAIT_ASYNC: usize = 0x5D;

// ── Network (0x60) ───────────────────────────────────────────────────

/// Create a network socket.
pub const SYS_NET_SOCKET: usize = 0x60;
/// Bind a socket to an address.
pub const SYS_NET_BIND: usize = 0x61;
/// Listen for connections.
pub const SYS_NET_LISTEN: usize = 0x62;
/// Accept a connection.
pub const SYS_NET_ACCEPT: usize = 0x63;
/// Connect to a remote address.
pub const SYS_NET_CONNECT: usize = 0x64;
/// Send a message on a socket.
pub const SYS_NET_SENDMSG: usize = 0x65;
/// Receive a message from a socket.
pub const SYS_NET_RECVMSG: usize = 0x66;
/// Shut down a socket.
pub const SYS_NET_SHUTDOWN: usize = 0x67;

// ── Device / IOMMU (0x70) ────────────────────────────────────────────

/// Create a BTI (Bus Transaction Initiator) from an IOMMU handle.
pub const SYS_BTI_CREATE: usize = 0x70;
/// Pin physical pages for DMA through a BTI.
pub const SYS_BTI_PIN: usize = 0x71;
/// Release BTI quarantine after error recovery.
pub const SYS_BTI_RELEASE_QUARANTINE: usize = 0x72;
/// Unpin a PMT (Pinned Memory Token), freeing the DMA mapping.
pub const SYS_PMT_UNPIN: usize = 0x73;

// ── System (0xF0) ────────────────────────────────────────────────────

/// Query system information.
pub const SYS_QUERY: usize = 0xF0;
/// Write to the kernel debug log (serial).
pub const SYS_DEBUG_LOG: usize = 0xF1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_duplicate_syscall_numbers() {
        let all: &[usize] = &[
            // Task
            SYS_TASK_EXIT,
            SYS_TASK_SPAWN,
            SYS_TASK_WAIT,
            SYS_TASK_KILL,
            SYS_TASK_CLONE,
            SYS_TASK_INFO,
            SYS_TASK_SIGACTION,
            SYS_TASK_SIGRETURN,
            SYS_TASK_SETPGID,
            SYS_TASK_GETPGID,
            SYS_TASK_GETPPID,
            SYS_TASK_GETCWD,
            SYS_TASK_CHDIR,
            SYS_TASK_SETSID,
            SYS_TASK_SIGPROCMASK,
            SYS_TASK_EXECVE,
            // Handle
            SYS_HANDLE_CLOSE,
            SYS_HANDLE_DUP,
            SYS_HANDLE_DUP_LOWEST,
            SYS_HANDLE_PIPE,
            SYS_HANDLE_TCSETPGRP,
            SYS_HANDLE_TCGETPGRP,
            SYS_HANDLE_IOCTL,
            SYS_HANDLE_FCNTL,
            SYS_HANDLE_PIPE2,
            // Channel
            SYS_CHANNEL_CREATE,
            SYS_CHANNEL_SEND,
            SYS_CHANNEL_RECV,
            SYS_CHANNEL_ACCEPT,
            SYS_CHANNEL_SEND_FD,
            SYS_CHANNEL_RECV_FD,
            // Vnode
            SYS_VNODE_OPEN,
            SYS_VNODE_READ,
            SYS_VNODE_WRITE,
            SYS_VNODE_STAT,
            SYS_VNODE_READDIR,
            SYS_VNODE_UNLINK,
            SYS_VNODE_SEEK,
            SYS_VNODE_MKDIR,
            SYS_VNODE_RENAME,
            SYS_VNODE_SYMLINK,
            SYS_VNODE_LINK,
            SYS_VNODE_READLINK,
            SYS_VNODE_TRUNCATE,
            SYS_VNODE_FSTATAT,
            SYS_VFS_MOUNT,
            SYS_VFS_UNMOUNT,
            // Memory
            SYS_MEM_MAP,
            SYS_MEM_UNMAP,
            SYS_MEM_BRK,
            SYS_MEM_CREATE_SHARED,
            SYS_MEM_MAP_SHARED,
            SYS_MEM_PROTECT,
            SYS_VMO_GET_SIZE,
            // Event
            SYS_EVENT_CREATE,
            SYS_EVENT_SIGNAL,
            SYS_EVENT_WAIT,
            SYS_EVENT_WAIT_MANY,
            SYS_CLOCK_GETTIME,
            SYS_CLOCK_NANOSLEEP,
            SYS_FUTEX,
            // Port / Timer
            SYS_PORT_CREATE,
            SYS_PORT_WAIT,
            SYS_PORT_QUEUE,
            SYS_TIMER_CREATE,
            SYS_TIMER_SET,
            SYS_TIMER_CANCEL,
            SYS_OBJECT_WAIT_ASYNC,
            // Network
            SYS_NET_SOCKET,
            SYS_NET_BIND,
            SYS_NET_LISTEN,
            SYS_NET_ACCEPT,
            SYS_NET_CONNECT,
            SYS_NET_SENDMSG,
            SYS_NET_RECVMSG,
            SYS_NET_SHUTDOWN,
            // Device / IOMMU
            SYS_BTI_CREATE,
            SYS_BTI_PIN,
            SYS_BTI_RELEASE_QUARANTINE,
            SYS_PMT_UNPIN,
            // System
            SYS_QUERY,
            SYS_DEBUG_LOG,
        ];

        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                if i != j {
                    assert_ne!(
                        a, b,
                        "duplicate syscall number {a:#x} at indices {i} and {j}"
                    );
                }
            }
        }
    }
}
