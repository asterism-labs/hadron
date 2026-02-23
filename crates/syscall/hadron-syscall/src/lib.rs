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
        /// `ESRCH` — no such process.
        ESRCH = 3;
        /// `EINTR` — interrupted system call.
        EINTR = 4;
        /// `ECHILD` — no child processes.
        ECHILD = 10;
        /// `EIO` — I/O error.
        EIO = 5;
        /// `EBADF` — bad file descriptor.
        EBADF = 9;
        /// `ENOMEM` — out of memory.
        ENOMEM = 12;
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
        /// `ESPIPE` — illegal seek (e.g. on a pipe or socket).
        ESPIPE = 29;
        /// `EPIPE` — broken pipe.
        EPIPE = 32;
        /// `ENAMETOOLONG` — file name too long.
        ENAMETOOLONG = 36;
        /// `ENOSYS` — function not implemented.
        ENOSYS = 38;
        /// `ELOOP` — too many levels of symbolic links.
        ELOOP = 40;
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

        /// Response for [`QUERY_PROCESSES`]: process table statistics.
        #[derive(Debug, Clone, Copy)]
        struct ProcessInfo {
            /// Number of active processes.
            count: u32,
            /// Padding for alignment.
            _pad: u32,
        }

        /// Stat information for a vnode.
        #[derive(Debug, Clone, Copy)]
        struct StatInfo {
            /// Inode type: 0=file, 1=directory, 2=chardev.
            inode_type: u8,
            /// Padding for alignment.
            _pad: [u8; 3],
            /// File size in bytes (0 for directories and devices).
            size: u64,
            /// Permissions: bit 0=read, bit 1=write, bit 2=exec.
            permissions: u32,
        }

        /// Framebuffer information returned by `FBIOGET_INFO` ioctl.
        #[derive(Debug, Clone, Copy)]
        struct FbInfo {
            /// Width in pixels.
            width: u32,
            /// Height in pixels.
            height: u32,
            /// Bytes per scanline.
            pitch: u32,
            /// Bits per pixel (widened from u8 for alignment).
            bpp: u32,
            /// Pixel format: 0=RGB32, 1=BGR32.
            pixel_format: u32,
        }

        /// Argument descriptor for [`task_spawn`]: pointer + length of one arg string.
        #[derive(Debug, Clone, Copy)]
        struct SpawnArg {
            /// Pointer to the argument string bytes.
            ptr: usize,
            /// Length of the argument string in bytes.
            len: usize,
        }

        /// A file descriptor mapping entry for [`SpawnInfo`].
        ///
        /// Maps a parent fd to a child fd number during `task_spawn`.
        #[derive(Debug, Clone, Copy)]
        struct FdMapEntry {
            /// Fd number in the child process.
            child_fd: u32,
            /// Fd number in the parent process to duplicate.
            parent_fd: u32,
        }

        /// Extended spawn information for [`task_spawn`].
        ///
        /// Passed by pointer so that the ABI can be extended with new fields
        /// without changing the syscall argument count. The kernel validates
        /// `info_len >= size_of::<SpawnInfo>()`.
        #[derive(Debug, Clone, Copy)]
        struct SpawnInfo {
            /// Pointer to the path string bytes.
            path_ptr: usize,
            /// Length of the path string.
            path_len: usize,
            /// Pointer to an array of [`SpawnArg`] descriptors for argv.
            argv_ptr: usize,
            /// Number of argv entries.
            argv_count: usize,
            /// Pointer to an array of [`SpawnArg`] descriptors for envp.
            /// Each envp entry is a `KEY=value` string.
            envp_ptr: usize,
            /// Number of envp entries.
            envp_count: usize,
            /// Pointer to an array of [`FdMapEntry`] for fd inheritance.
            /// If null (0), the default behavior applies (inherit fds 0/1/2).
            fd_map_ptr: usize,
            /// Number of fd map entries.
            fd_map_count: usize,
            /// Pointer to a CWD path string for the child process.
            /// If null (0), the child inherits the parent's CWD.
            cwd_ptr: usize,
            /// Length of the CWD path string.
            cwd_len: usize,
        }

        /// A poll descriptor for [`event_wait_many`].
        ///
        /// Describes one file descriptor to monitor. The kernel fills in
        /// `revents` with the events that actually occurred.
        #[derive(Debug, Clone, Copy)]
        struct PollFd {
            /// File descriptor to monitor.
            fd: u32,
            /// Requested events bitmask (POLLIN, POLLOUT, etc.).
            events: u16,
            /// Returned events bitmask (filled by kernel).
            revents: u16,
        }

        /// Terminal I/O settings (POSIX `termios`).
        ///
        /// Controls line discipline behavior: canonical vs raw mode, echo,
        /// signal generation, and special character handling.
        #[derive(Debug, Clone, Copy)]
        struct Termios {
            /// Input mode flags.
            iflag: u32,
            /// Output mode flags.
            oflag: u32,
            /// Control mode flags.
            cflag: u32,
            /// Local mode flags.
            lflag: u32,
            /// Special control characters (indexed by VEOF, VINTR, etc.).
            cc: [u8; 32],
        }

        /// Window size for `TIOCGWINSZ` / `TIOCSWINSZ`.
        #[derive(Debug, Clone, Copy)]
        struct Winsize {
            /// Number of rows (characters).
            rows: u16,
            /// Number of columns (characters).
            cols: u16,
            /// Horizontal size in pixels (informational).
            xpixel: u16,
            /// Vertical size in pixels (informational).
            ypixel: u16,
        }

        /// A single directory entry returned by `vnode_readdir`.
        #[derive(Debug, Clone, Copy)]
        struct DirEntryInfo {
            /// Inode type: 0=file, 1=directory, 2=chardev.
            inode_type: u8,
            /// Length of the name (bytes used in `name` array).
            name_len: u8,
            /// Padding for alignment.
            _pad: [u8; 2],
            /// Entry name as UTF-8 bytes, not NUL-terminated.
            name: [u8; 60],
        }
    }

    constants {
        /// Query topic: physical memory statistics.
        QUERY_MEMORY: u64 = 0;
        /// Query topic: time since boot.
        QUERY_UPTIME: u64 = 1;
        /// Query topic: kernel version information.
        QUERY_KERNEL_VERSION: u64 = 2;
        /// Query topic: process table statistics.
        QUERY_PROCESSES: u64 = 3;
        /// Monotonic clock: nanoseconds since boot, never adjusted.
        CLOCK_MONOTONIC: usize = 0;
        /// Real-time clock: Unix epoch seconds (wall-clock time).
        CLOCK_REALTIME: usize = 1;
        /// Inode type: regular file.
        INODE_TYPE_FILE: u8 = 0;
        /// Inode type: directory.
        INODE_TYPE_DIR: u8 = 1;
        /// Inode type: character device.
        INODE_TYPE_CHARDEV: u8 = 2;
        /// Inode type: symbolic link.
        INODE_TYPE_SYMLINK: u8 = 3;
        /// Memory protection: allow reads.
        PROT_READ: usize = 0x1;
        /// Memory protection: allow writes.
        PROT_WRITE: usize = 0x2;
        /// Memory protection: allow execution.
        PROT_EXEC: usize = 0x4;
        /// Memory mapping flag: anonymous (not file-backed).
        MAP_ANONYMOUS: usize = 0x1;
        /// Memory mapping flag: shared/device-backed mapping.
        MAP_SHARED: usize = 0x2;
        /// Open flag: open for reading.
        OPEN_READ: usize = 0x0001;
        /// Open flag: open for writing.
        OPEN_WRITE: usize = 0x0002;
        /// Open flag: create file if it does not exist.
        OPEN_CREATE: usize = 0x0004;
        /// Open flag: truncate file to zero length.
        OPEN_TRUNCATE: usize = 0x0008;
        /// Open flag: writes always append to end of file.
        OPEN_APPEND: usize = 0x0010;
        /// Open flag: close on exec.
        OPEN_CLOEXEC: usize = 0x0020;
        /// Open flag: non-blocking I/O.
        OPEN_NONBLOCK: usize = 0x0040;
        /// Open flag: fail if not a directory.
        OPEN_DIRECTORY: usize = 0x0080;
        /// Open flag: fail if file already exists (with `OPEN_CREATE`).
        OPEN_EXCL: usize = 0x0100;
        /// Open flag: do not follow symbolic links.
        OPEN_NOFOLLOW: usize = 0x0200;
        /// Special dirfd value: use current working directory.
        AT_FDCWD: usize = 0xFFFF_FF9C;
        /// `fstatat` flag: do not follow symbolic links.
        AT_SYMLINK_NOFOLLOW: usize = 0x100;
        /// Seek from beginning of file.
        SEEK_SET: usize = 0;
        /// Seek from current position.
        SEEK_CUR: usize = 1;
        /// Seek from end of file.
        SEEK_END: usize = 2;
        /// `fcntl` command: duplicate fd to lowest free fd >= arg.
        F_DUPFD: usize = 0;
        /// `fcntl` command: get fd flags (`FD_CLOEXEC`).
        F_GETFD: usize = 1;
        /// `fcntl` command: set fd flags (`FD_CLOEXEC`).
        F_SETFD: usize = 2;
        /// `fcntl` command: get file status flags (`O_NONBLOCK`, `O_APPEND`).
        F_GETFL: usize = 3;
        /// `fcntl` command: set file status flags (`O_NONBLOCK`, `O_APPEND`).
        F_SETFL: usize = 4;
        /// `fcntl` command: duplicate fd to lowest free fd >= arg, with `CLOEXEC`.
        F_DUPFD_CLOEXEC: usize = 0x406;
        /// File descriptor flag: close on exec.
        FD_CLOEXEC: usize = 1;
        /// `sigprocmask` how: block signals in set.
        SIG_BLOCK: usize = 0;
        /// `sigprocmask` how: unblock signals in set.
        SIG_UNBLOCK: usize = 1;
        /// `sigprocmask` how: replace mask with set.
        SIG_SETMASK: usize = 2;
        /// Pipe2 flag: set `O_CLOEXEC` on both pipe fds.
        PIPE_CLOEXEC: usize = 0x0020;
        /// Pipe2 flag: set `O_NONBLOCK` on both pipe fds.
        PIPE_NONBLOCK: usize = 0x0040;
        /// Framebuffer ioctl: get framebuffer info.
        FBIOGET_INFO: u32 = 0x4600;
        /// Signal: interrupt (Ctrl+C).
        SIGINT: usize = 2;
        /// Signal: quit (Ctrl+\).
        SIGQUIT: usize = 3;
        /// Signal: kill (cannot be caught or ignored).
        SIGKILL: usize = 9;
        /// Signal: segmentation fault.
        SIGSEGV: usize = 11;
        /// Signal: broken pipe.
        SIGPIPE: usize = 13;
        /// Signal: terminate.
        SIGTERM: usize = 15;
        /// Signal: child process exited.
        SIGCHLD: usize = 17;
        /// Signal: stop (cannot be caught or ignored).
        SIGSTOP: usize = 19;
        /// Signal disposition: default action.
        SIG_DFL: usize = 0;
        /// Signal disposition: ignore the signal.
        SIG_IGN: usize = 1;
        /// Sigaction flag: restart interrupted syscalls after handler returns.
        SA_RESTART: usize = 0x1000_0000;
        /// Sigaction flag: reset handler to `SIG_DFL` after delivery.
        SA_RESETHAND: usize = 0x8000_0000;
        /// `task_wait` flag: return immediately if no child has exited.
        WNOHANG: usize = 1;
        /// `task_wait` flag: also report stopped children.
        WUNTRACED: usize = 2;
        /// Poll event: data available for reading.
        POLLIN: u16 = 0x0001;
        /// Poll event: writing will not block.
        POLLOUT: u16 = 0x0004;
        /// Poll event: error condition (revents only).
        POLLERR: u16 = 0x0008;
        /// Poll event: hang-up (revents only).
        POLLHUP: u16 = 0x0010;
        /// Poll event: invalid fd (revents only).
        POLLNVAL: u16 = 0x0020;
        /// Termios ioctl: get terminal attributes.
        TCGETS: u32 = 0x5401;
        /// Termios ioctl: set terminal attributes immediately.
        TCSETS: u32 = 0x5402;
        /// Termios ioctl: set terminal attributes after output drains.
        TCSETSW: u32 = 0x5403;
        /// Termios ioctl: set terminal attributes after output drains, discard input.
        TCSETSF: u32 = 0x5404;
        /// Termios ioctl: get foreground process group.
        TIOCGPGRP: u32 = 0x540F;
        /// Termios ioctl: set foreground process group.
        TIOCSPGRP: u32 = 0x5410;
        /// Termios ioctl: get window size.
        TIOCGWINSZ: u32 = 0x5413;
        /// Termios ioctl: set window size.
        TIOCSWINSZ: u32 = 0x5414;
        /// Termios lflag: enable canonical (line-editing) mode.
        ICANON: u32 = 0x0002;
        /// Termios lflag: echo input characters.
        ECHO: u32 = 0x0008;
        /// Termios lflag: echo newline even if ECHO is off.
        ECHONL: u32 = 0x0040;
        /// Termios lflag: generate signals (SIGINT, SIGQUIT, SIGTSTP).
        ISIG: u32 = 0x0001;
        /// Termios iflag: translate CR to NL on input.
        ICRNL: u32 = 0x0100;
        /// Termios oflag: post-process output.
        OPOST: u32 = 0x0001;
        /// Termios oflag: map NL to CR-NL on output.
        ONLCR: u32 = 0x0004;
        /// Index into `cc` array: EOF character (default Ctrl+D = 0x04).
        VEOF: usize = 4;
        /// Index into `cc` array: EOL character.
        VEOL: usize = 11;
        /// Index into `cc` array: erase character (default Ctrl+H = 0x08).
        VERASE: usize = 2;
        /// Index into `cc` array: interrupt character (default Ctrl+C = 0x03).
        VINTR: usize = 0;
        /// Index into `cc` array: kill line character (default Ctrl+U = 0x15).
        VKILL: usize = 3;
        /// Index into `cc` array: minimum bytes for non-canonical read.
        VMIN: usize = 6;
        /// Index into `cc` array: quit character (default Ctrl+\ = 0x1C).
        VQUIT: usize = 1;
        /// Index into `cc` array: timeout for non-canonical read (tenths of sec).
        VTIME: usize = 5;
        /// Clone flag: share address space (threads).
        CLONE_VM: usize = 0x0100;
        /// Clone flag: share file descriptor table.
        CLONE_FILES: usize = 0x0400;
        /// Clone flag: share signal handlers.
        CLONE_SIGHAND: usize = 0x0800;
        /// Clone flag: set the child's TLS (FS_BASE).
        CLONE_SETTLS: usize = 0x0008_0000;
        /// Futex operation: sleep if `*addr == val`.
        FUTEX_WAIT: usize = 0;
        /// Futex operation: wake up to `val` waiters.
        FUTEX_WAKE: usize = 1;
        /// PTY ioctl: get slave PTY number.
        TIOCGPTN: u32 = 0x5430;
        /// PTY ioctl: unlock slave PTY.
        TIOCSPTLCK: u32 = 0x5431;
    }

    /// Task management.
    group task(0x00..0x10) {
        /// Terminate the current task.
        fn task_exit(status: usize) = 0x00;

        /// Spawn a new task from an ELF binary.
        ///
        /// `info_ptr` points to a [`SpawnInfo`] struct. `info_len` must be
        /// at least `size_of::<SpawnInfo>()`. The struct contains path, argv,
        /// and envp descriptors.
        fn task_spawn(info_ptr: usize, info_len: usize) = 0x01;

        /// Wait for a child task to exit. Returns child PID on success.
        ///
        /// `flags` is a bitmask of `WNOHANG` (non-blocking) and `WUNTRACED`
        /// (also report stopped children). Pass 0 for default blocking wait.
        fn task_wait(pid: usize, status_ptr: usize, flags: usize) = 0x02;

        /// Send a signal to a task.
        ///
        /// `pid` is the target process ID, `signum` is the signal number.
        /// Returns 0 on success, or a negated errno on failure.
        fn task_kill(pid: usize, signum: usize) = 0x03;

        /// Clone the current task (create a thread).
        ///
        /// `flags` is a bitmask of `CLONE_VM`, `CLONE_FILES`, `CLONE_SIGHAND`.
        /// `stack_ptr` is the top of the new thread's user stack.
        /// `tls_ptr` is the thread-local storage base (written to FS_BASE).
        /// Returns the new thread's TID in the parent, 0 in the child.
        fn task_clone(flags: usize, stack_ptr: usize, tls_ptr: usize) = 0x04;

        /// Query task information (returns task ID for now).
        fn task_info() = 0x05;

        /// Register a signal handler for a signal number.
        ///
        /// `signum` is the signal number (1-63, cannot be SIGKILL or SIGSTOP).
        /// `handler` is `SIG_DFL` (0), `SIG_IGN` (1), or a function pointer.
        /// `flags` is a bitmask of `SA_RESTART`, `SA_RESETHAND`, etc.
        /// If `old_handler_out` is non-zero, the previous handler is written there.
        /// Returns 0 on success, or a negated errno on failure.
        fn task_sigaction(signum: usize, handler: usize, flags: usize, old_handler_out: usize) = 0x06;

        /// Restore pre-signal user context after a signal handler returns.
        ///
        /// Called from the signal trampoline. Restores all registers from
        /// the `SignalFrame` saved on the user stack before re-entering
        /// userspace at the interrupted instruction.
        fn task_sigreturn() = 0x07;

        /// Set process group ID.
        ///
        /// If `pid` is 0, uses the calling process. If `pgid` is 0, uses `pid`
        /// as the new PGID (creating a new process group). The target must be
        /// the caller or a child of the caller.
        fn task_setpgid(pid: usize, pgid: usize) = 0x08;

        /// Get process group ID.
        ///
        /// If `pid` is 0, returns the calling process's PGID.
        fn task_getpgid(pid: usize) = 0x09;

        /// Get parent process ID.
        ///
        /// Returns the PID of the calling process's parent, or 0 if the process
        /// has no parent (e.g. the init process).
        fn task_getppid() = 0x0A;

        /// Get current working directory.
        ///
        /// Copies the CWD path into the user buffer at `(buf_ptr, buf_len)`.
        /// Returns the length of the CWD string on success, or a negated errno.
        fn task_getcwd(buf_ptr: usize, buf_len: usize) = 0x0B;

        /// Change current working directory.
        ///
        /// `path_ptr` and `path_len` describe the new working directory.
        /// Returns 0 on success, or a negated errno on failure.
        fn task_chdir(path_ptr: usize, path_len: usize) = 0x0C;

        /// Create a new session.
        ///
        /// The calling process becomes the session leader and process group leader.
        /// Returns the new session ID on success, or a negated errno on failure.
        fn task_setsid() = 0x0D;

        /// Set the signal mask for the calling process.
        ///
        /// `how` is `SIG_BLOCK`, `SIG_UNBLOCK`, or `SIG_SETMASK`.
        /// `set` is the signal set to apply. `oldset_out` receives the
        /// previous mask if non-zero. Returns 0 on success.
        fn task_sigprocmask(how: usize, set: usize, oldset_out: usize) = 0x0E;

        /// Replace the current process image with a new program.
        ///
        /// `info_ptr` points to a [`SpawnInfo`] struct containing the new
        /// program path, argv, and envp. The PID, parent, fd table, and CWD
        /// are preserved. Signal handlers are reset to `SIG_DFL`.
        /// Does not return on success. Returns negated errno on failure.
        fn task_execve(info_ptr: usize, info_len: usize) = 0x0F;
    }

    /// Handle operations.
    group handle(0x10..0x20) {
        /// Close a handle (file descriptor).
        fn handle_close(handle: usize) = 0x00;

        /// Duplicate a handle (dup2 semantics): copy `old_fd` to `new_fd`.
        fn handle_dup(old_fd: usize, new_fd: usize) = 0x01;

        /// Duplicate a handle to the lowest available fd number.
        ///
        /// Returns the new fd on success, or a negated errno on failure.
        fn handle_dup_lowest(old_fd: usize) = 0x02;

        /// Create a pipe. Writes [read_fd, write_fd] to `fds_ptr`.
        fn handle_pipe(fds_ptr: usize) = 0x03;

        /// Set the foreground process group of the terminal associated with `fd`.
        ///
        /// `pgid` is the process group ID to set. Returns 0 on success, or a
        /// negated errno on failure.
        fn handle_tcsetpgrp(fd: usize, pgid: usize) = 0x04;

        /// Get the foreground process group of the terminal associated with `fd`.
        ///
        /// Returns the PGID on success, or a negated errno on failure.
        fn handle_tcgetpgrp(fd: usize) = 0x05;

        /// Perform a device-specific ioctl on a file descriptor.
        ///
        /// `cmd` is the ioctl command number. `arg_ptr` is a pointer to the
        /// command-specific argument (typically a `#[repr(C)]` struct).
        /// Returns 0 on success, or a negated errno on failure.
        fn handle_ioctl(fd: usize, cmd: usize, arg_ptr: usize) = 0x06;

        /// Perform an fcntl operation on a file descriptor.
        ///
        /// `cmd` is one of `F_DUPFD`, `F_GETFD`, `F_SETFD`, `F_GETFL`,
        /// `F_SETFL`, or `F_DUPFD_CLOEXEC`. `arg` is command-specific.
        /// Returns a value dependent on the command, or a negated errno.
        fn handle_fcntl(fd: usize, cmd: usize, arg: usize) = 0x07;

        /// Create a pipe with flags. Writes [read_fd, write_fd] to `fds_ptr`.
        ///
        /// `flags` may include `PIPE_CLOEXEC` and/or `PIPE_NONBLOCK`.
        fn handle_pipe2(fds_ptr: usize, flags: usize) = 0x08;
    }

    /// Channel IPC.
    group channel(0x20..0x30) {
        /// Create a bidirectional channel pair.
        ///
        /// Writes two file descriptor numbers `[fd_a, fd_b]` to the user
        /// buffer at `fds_ptr`. Each endpoint can send and receive messages.
        fn channel_create(fds_ptr: usize) = 0x00;

        /// Send a message on a channel endpoint.
        ///
        /// `handle` is the channel fd. `buf_ptr`/`buf_len` describe the
        /// message to send (max 4096 bytes). Messages are discrete — not
        /// a byte stream. Blocks if the send queue is full.
        fn channel_send(handle: usize, buf_ptr: usize, buf_len: usize) = 0x01;

        /// Receive a message from a channel endpoint.
        ///
        /// `handle` is the channel fd. The next queued message is copied
        /// into `(buf_ptr, buf_len)`. Returns the message length (truncated
        /// if the buffer is smaller). Blocks if the receive queue is empty.
        fn channel_recv(handle: usize, buf_ptr: usize, buf_len: usize) = 0x02;

        /// Synchronous call on a channel.
        #[reserved]
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

        /// Stat a vnode: write [`StatInfo`] to `(buf_ptr, buf_len)`.
        fn vnode_stat(fd: usize, buf_ptr: usize, buf_len: usize) = 0x03;

        /// Read directory entries as [`DirEntryInfo`] array into `(buf_ptr, buf_len)`.
        fn vnode_readdir(fd: usize, buf_ptr: usize, buf_len: usize) = 0x04;

        /// Unlink a file or empty directory.
        ///
        /// `path_ptr` and `path_len` describe the path to unlink.
        /// Returns 0 on success, or a negated errno on failure.
        fn vnode_unlink(path_ptr: usize, path_len: usize) = 0x05;

        /// Seek to a position in a file.
        ///
        /// `whence` is one of `SEEK_SET`, `SEEK_CUR`, or `SEEK_END`.
        /// Returns the new absolute offset on success, or a negated errno.
        fn vnode_seek(fd: usize, offset: usize, whence: usize) = 0x06;

        /// Create a directory.
        ///
        /// `path_ptr` and `path_len` describe the directory path to create.
        /// `permissions` is a bitmask of read/write/execute permissions.
        /// Returns 0 on success, or a negated errno on failure.
        fn vnode_mkdir(path_ptr: usize, path_len: usize, permissions: usize) = 0x07;

        /// Rename (move) a file or directory.
        ///
        /// `old_ptr`/`old_len` is the source path; `new_ptr`/`new_len` is the
        /// destination path. Returns 0 on success, or a negated errno.
        fn vnode_rename(old_ptr: usize, old_len: usize, new_ptr: usize, new_len: usize) = 0x08;

        /// Create a symbolic link.
        ///
        /// `target_ptr`/`target_len` is the target path (what the link points to).
        /// `link_ptr`/`link_len` is the path of the new symlink.
        fn vnode_symlink(target_ptr: usize, target_len: usize, link_ptr: usize, link_len: usize) = 0x09;

        /// Create a hard link.
        ///
        /// `target_ptr`/`target_len` is the existing path.
        /// `link_ptr`/`link_len` is the new link path.
        fn vnode_link(target_ptr: usize, target_len: usize, link_ptr: usize, link_len: usize) = 0x0A;

        /// Read the target of a symbolic link.
        ///
        /// Copies the symlink target into `(buf_ptr, buf_len)`.
        /// Returns the length of the target string, or a negated errno.
        fn vnode_readlink(path_ptr: usize, path_len: usize, buf_ptr: usize, buf_len: usize) = 0x0B;

        /// Truncate a file to a specified length.
        ///
        /// If `len` is less than the current size, data is lost.
        /// If `len` is greater, the file is extended with zero bytes.
        fn vnode_truncate(fd: usize, len: usize) = 0x0C;

        /// Stat a file relative to a directory fd.
        ///
        /// `dirfd` is the base directory fd (or `AT_FDCWD` for CWD).
        /// `path_ptr`/`path_len` is the relative path. `buf` receives the
        /// stat result. `flags` is a bitmask of `AT_SYMLINK_NOFOLLOW` etc.
        fn vnode_fstatat(
            dirfd: usize,
            path_ptr: usize,
            path_len: usize,
            buf: usize,
            flags: usize,
        ) = 0x0D;
    }

    /// Memory management.
    group memory(0x40..0x50) {
        /// Map memory into the address space.
        ///
        /// `addr_hint` is ignored (kernel chooses address). `length` is the
        /// requested size in bytes (rounded up to page alignment). `prot` is
        /// a bitmask of `PROT_READ`/`PROT_WRITE`/`PROT_EXEC`. `flags` must
        /// include `MAP_ANONYMOUS` or `MAP_SHARED`. `fd` is the file
        /// descriptor for device-backed mappings (ignored for anonymous).
        ///
        /// Returns the mapped virtual address on success, or negated errno.
        fn mem_map(addr_hint: usize, length: usize, prot: usize, flags: usize, fd: usize) = 0x00;

        /// Unmap memory from the address space.
        ///
        /// `addr` must be the exact address returned by `mem_map`. `length`
        /// must match the original mapping size.
        fn mem_unmap(addr: usize, length: usize) = 0x01;

        /// Adjust the program break (heap boundary).
        ///
        /// If `addr` is 0, returns the current break address.
        /// If `addr` is greater than the current break, the heap is expanded.
        /// If `addr` is less than the current break, the heap is shrunk.
        /// Returns the new break address on success, or a negated errno.
        fn mem_brk(addr: usize) = 0x02;

        /// Create a shared memory object.
        ///
        /// Allocates `size` bytes of physical memory (page-aligned) and
        /// returns a file descriptor referring to the shared memory object.
        /// The memory is zero-filled. Multiple processes can map the same
        /// object to share memory.
        fn mem_create_shared(size: usize) = 0x03;

        /// Map a shared memory object into the calling process's address space.
        ///
        /// `fd` is a shared memory fd from [`mem_create_shared`]. `size` is
        /// the mapping length (must not exceed the object size). `prot` is a
        /// bitmask of `PROT_READ`/`PROT_WRITE`.
        /// Returns the mapped virtual address on success, or negated errno.
        fn mem_map_shared(fd: usize, size: usize, prot: usize) = 0x04;
    }

    /// Events and time.
    group event(0x50..0x60) {
        /// Create an event object.
        #[reserved]
        fn event_create() = 0x00;

        /// Signal an event.
        #[reserved]
        fn event_signal(handle: usize) = 0x01;

        /// Wait for an event.
        #[reserved]
        fn event_wait(handle: usize) = 0x02;

        /// Poll multiple file descriptors for readiness.
        ///
        /// `fds_ptr` is a user pointer to an array of [`PollFd`] structs.
        /// `nfds` is the number of entries. `timeout_ms` is the timeout in
        /// milliseconds (`usize::MAX` for infinite, `0` for non-blocking).
        ///
        /// Returns the number of fds with non-zero `revents`, or negated errno.
        fn event_wait_many(fds_ptr: usize, nfds: usize, timeout_ms: usize) = 0x03;

        /// Get current time.
        fn clock_gettime(clock_id: usize, tp: usize) = 0x04;

        /// High-resolution sleep.
        ///
        /// Suspends the calling task for the duration specified in the
        /// [`Timespec`] at `req_ptr`. `clock_id` selects the clock.
        /// `flags` is reserved (must be 0). If `rem_ptr` is non-zero and
        /// the sleep is interrupted, the remaining time is written there.
        /// Returns 0 on success, or a negated errno on failure.
        fn clock_nanosleep(clock_id: usize, flags: usize, req_ptr: usize, rem_ptr: usize) = 0x05;

        /// Futex — fast userspace mutex primitive.
        ///
        /// `addr` is the userspace address of a `u32` futex word.
        /// `op` is `FUTEX_WAIT` or `FUTEX_WAKE`.
        /// `val` is the expected value (for WAIT) or count to wake (for WAKE).
        /// `timeout_ms` is the timeout in ms (0 = infinite, only for WAIT).
        ///
        /// FUTEX_WAIT: if `*addr == val`, sleep until woken or timeout.
        /// FUTEX_WAKE: wake up to `val` threads sleeping on `addr`.
        fn futex(addr: usize, op: usize, val: usize, timeout_ms: usize) = 0x06;
    }

    /// System services.
    group system(0xF0..0x100) {
        /// Query system information via typed `#[repr(C)]` response structs.
        fn query(topic: usize, sub_id: usize, out_buf: usize, out_len: usize) = 0x00;

        /// Write a debug message to the kernel serial console.
        fn debug_log(buf: usize, len: usize) = 0x01;
    }
}
