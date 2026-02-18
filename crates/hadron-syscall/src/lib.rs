//! Single source of truth for Hadron syscall definitions.
//!
//! This crate uses the `define_syscalls!` proc macro to generate:
//! - Syscall number constants (`SYS_*`)
//! - Error code constants (`E*`)
//! - `#[repr(C)]` data structures shared between kernel and userspace
//! - Named constants (query topics, clock IDs)
//! - `Syscall` and `SyscallGroup` enums with introspection methods
//! - (feature `kernel`) `SyscallHandler` trait and `dispatch()` function
//! - (feature `userspace`) Raw `syscallN` asm stubs and typed wrapper functions

#![no_std]

hadron_syscall_macros::define_syscalls! {
    errors {
        /// `ENOENT` — no such file or directory.
        ENOENT = 2;
        /// `EIO` — I/O error.
        EIO = 5;
        /// `EBADF` — bad file descriptor.
        EBADF = 9;
        /// `EACCES` — permission denied.
        EACCES = 13;
        /// `EFAULT` — bad address.
        EFAULT = 14;
        /// `EEXIST` — file exists.
        EEXIST = 17;
        /// `ENOTDIR` — not a directory.
        ENOTDIR = 20;
        /// `EISDIR` — is a directory.
        EISDIR = 21;
        /// `EINVAL` — invalid argument.
        EINVAL = 22;
        /// `ENOSYS` — function not implemented.
        ENOSYS = 38;
    }

    types {
        /// POSIX-compatible timespec for `clock_gettime` results.
        ///
        /// Uses `u64` fields (not `i64`) because Hadron only supports monotonic
        /// boot-relative time — negative timestamps are impossible.
        #[derive(Debug, Clone, Copy)]
        struct Timespec {
            /// Seconds since boot.
            tv_sec: u64,
            /// Nanoseconds within the current second (`0..999_999_999`).
            tv_nsec: u64,
        }

        /// Response for [`QUERY_MEMORY`]: physical memory statistics.
        #[derive(Debug, Clone, Copy)]
        struct MemoryInfo {
            /// Total physical memory in bytes.
            total_bytes: u64,
            /// Free physical memory in bytes.
            free_bytes: u64,
            /// Used physical memory in bytes (`total_bytes - free_bytes`).
            used_bytes: u64,
        }

        /// Response for [`QUERY_UPTIME`]: time elapsed since boot.
        #[derive(Debug, Clone, Copy)]
        struct UptimeInfo {
            /// Nanoseconds since boot.
            uptime_ns: u64,
        }

        /// Response for [`QUERY_KERNEL_VERSION`]: kernel version metadata.
        #[derive(Debug, Clone, Copy)]
        struct KernelVersionInfo {
            /// Major version number.
            major: u16,
            /// Minor version number.
            minor: u16,
            /// Patch version number.
            patch: u16,
            /// Padding for alignment.
            _pad: u16,
            /// Kernel name as a UTF-8 byte array, NUL-padded.
            name: [u8; 32],
        }
    }

    constants {
        /// Query topic: physical memory statistics.
        QUERY_MEMORY: u64 = 0;
        /// Query topic: time since boot.
        QUERY_UPTIME: u64 = 1;
        /// Query topic: kernel version information.
        QUERY_KERNEL_VERSION: u64 = 2;
        /// Monotonic clock: nanoseconds since boot, never adjusted.
        CLOCK_MONOTONIC: usize = 0;
    }

    /// Task management.
    group task(0x00..0x10) {
        /// Terminate the current task.
        fn task_exit(status: usize) = 0x00;

        /// Spawn a new task.
        #[reserved(phase = 9)]
        fn task_spawn() = 0x01;

        /// Wait for a task to exit.
        #[reserved(phase = 11)]
        fn task_wait() = 0x02;

        /// Kill a task.
        #[reserved(phase = 11)]
        fn task_kill() = 0x03;

        /// Detach a task.
        #[reserved(phase = 11)]
        fn task_detach() = 0x04;

        /// Query task information (returns task ID for now).
        fn task_info() = 0x05;
    }

    /// Handle operations.
    group handle(0x10..0x20) {
        /// Close a handle.
        #[reserved(phase = 11)]
        fn handle_close(handle: usize) = 0x00;

        /// Duplicate a handle.
        #[reserved(phase = 11)]
        fn handle_dup(handle: usize) = 0x01;

        /// Query handle info.
        #[reserved(phase = 11)]
        fn handle_info(handle: usize) = 0x02;
    }

    /// Channel IPC.
    group channel(0x20..0x30) {
        /// Create a channel pair.
        #[reserved(phase = 11)]
        fn channel_create() = 0x00;

        /// Send a message on a channel.
        #[reserved(phase = 11)]
        fn channel_send(handle: usize, buf_ptr: usize, buf_len: usize) = 0x01;

        /// Receive a message from a channel.
        #[reserved(phase = 11)]
        fn channel_recv(handle: usize, buf_ptr: usize, buf_len: usize) = 0x02;

        /// Synchronous call on a channel.
        #[reserved(phase = 11)]
        fn channel_call(handle: usize, buf_ptr: usize, buf_len: usize) = 0x03;
    }

    /// Filesystem / vnodes.
    group vnode(0x30..0x40) {
        /// Open a vnode by path.
        fn vnode_open(path_ptr: usize, path_len: usize, flags: usize) = 0x00;

        /// Read from a vnode.
        fn vnode_read(fd: usize, buf_ptr: usize, buf_len: usize) = 0x01;

        /// Write to a vnode.
        fn vnode_write(fd: usize, buf_ptr: usize, buf_len: usize) = 0x02;

        /// Stat a vnode.
        #[reserved(phase = 10)]
        fn vnode_stat(fd: usize) = 0x03;

        /// Read directory entries.
        #[reserved(phase = 10)]
        fn vnode_readdir(fd: usize, buf_ptr: usize, buf_len: usize) = 0x04;

        /// Unlink a vnode.
        #[reserved(phase = 10)]
        fn vnode_unlink(path_ptr: usize, path_len: usize) = 0x05;
    }

    /// Memory management.
    group memory(0x40..0x50) {
        /// Map memory into the address space.
        fn mem_map() = 0x00;

        /// Unmap memory from the address space.
        fn mem_unmap() = 0x01;

        /// Change memory protection flags.
        #[reserved(phase = 9)]
        fn mem_protect() = 0x02;

        /// Create a shared memory object.
        #[reserved(phase = 11)]
        fn mem_create_shared() = 0x03;

        /// Map a shared memory object.
        #[reserved(phase = 11)]
        fn mem_map_shared() = 0x04;
    }

    /// Events and time.
    group event(0x50..0x60) {
        /// Create an event object.
        #[reserved(phase = 11)]
        fn event_create() = 0x00;

        /// Signal an event.
        #[reserved(phase = 11)]
        fn event_signal(handle: usize) = 0x01;

        /// Wait for an event.
        #[reserved(phase = 11)]
        fn event_wait(handle: usize) = 0x02;

        /// Wait for multiple events.
        #[reserved(phase = 11)]
        fn event_wait_many(handles_ptr: usize, handles_len: usize) = 0x03;

        /// Get current time.
        fn clock_gettime(clock_id: usize, tp: usize) = 0x04;

        /// Create a timer.
        #[reserved(phase = 11)]
        fn timer_create() = 0x05;
    }

    /// System services.
    group system(0xF0..0x100) {
        /// Query system information via typed `#[repr(C)]` response structs.
        fn query(topic: usize, sub_id: usize, out_buf: usize, out_len: usize) = 0x00;

        /// Write a debug message to the kernel serial console.
        fn debug_log(buf: usize, len: usize) = 0x01;
    }
}
