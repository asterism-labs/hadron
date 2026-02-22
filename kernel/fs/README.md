# hadron-fs

Virtual filesystem layer for the Hadron kernel. This crate defines the `Inode` and `FileSystem` traits that abstract over different filesystem implementations (ramfs, devfs, FAT, ISO 9660, etc.) and provides the VFS mount table with path resolution. All file I/O in the kernel goes through these traits. Read and write operations return pinned boxed futures to support async I/O while remaining object-safe -- in-memory filesystems resolve in a single poll, while block-backed filesystems can yield to the executor.

## Features

- **Async-ready inode trait** -- the `Inode` trait provides `read`, `write`, `lookup`, `readdir`, `create`, and `unlink` operations as pinned boxed futures; in-memory implementations resolve immediately, block-backed implementations can `.await` on disk I/O
- **VFS mount table** -- maintains a `BTreeMap` of mount points keyed by path; path resolution finds the longest-matching mount prefix then walks remaining components via `Inode::lookup`
- **Symlink resolution** -- the VFS follows symbolic links during path resolution up to a configurable depth limit (8 levels) to prevent infinite loops
- **Device filesystem (devfs)** -- built-in `/dev/null` (reads return EOF, writes discarded), `/dev/zero` (reads fill with zeros), and extensible registration for additional device nodes like `/dev/console`
- **File descriptor table** -- per-process file descriptor management with open flags (read, write, append), offset tracking, and support for `dup`, `close`, and `pipe` operations
- **Path utilities** -- absolute path validation, component iteration, longest-prefix mount matching, and mount-prefix stripping
- **POSIX errno mapping** -- `FsError` variants map directly to POSIX errno values (`ENOENT`, `ENOTDIR`, `EISDIR`, `EEXIST`, `EBADF`, `EACCES`, `EIO`, `EINVAL`, `ENOSYS`, `ELOOP`, `EINTR`) for syscall return values
- **Synchronous polling helpers** -- `poll_immediate` for in-memory operations that must resolve in a single poll, and `try_poll_immediate` for syscall handlers that attempt sync I/O before falling back to the async TRAP_IO mechanism
