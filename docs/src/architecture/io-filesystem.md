# I/O & Filesystem

The Hadron kernel provides a Virtual Filesystem (VFS) layer that abstracts over different filesystem implementations behind a uniform set of traits. All file I/O -- whether targeting an in-memory ramfs, a device node in devfs, or a block-device-backed filesystem -- passes through the VFS. File descriptor management integrates with the VFS to provide standard POSIX-like file operations.

Source: [`kernel/kernel/src/fs/`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/fs/), [`kernel/drivers/src/fs/`](https://github.com/anomalyco/hadron/blob/main/kernel/drivers/src/fs/)

## VFS Architecture

The VFS has three logical layers:

```
Userspace syscalls (open, read, write, close, ...)
        |
  FileDescriptorTable   (per-process fd -> inode mapping)
        |
       Vfs              (mount table + path resolution)
      / | \
  RamFs DevFs BlockFs   (FileSystem implementations)
    |     |       |
 RamInode DevNull BlockDeviceAdapter -> BlockDevice trait
         DevZero
         DevConsole
         TTY
```

1. **Core traits** (`fs/mod.rs`) -- `Inode` and `FileSystem` define the interface every filesystem must implement.
2. **Mount table and path resolution** (`fs/vfs.rs`) -- A global `Vfs` struct maps mount paths to `FileSystem` instances and resolves absolute paths to inodes.
3. **File descriptors** (`fs/file.rs`) -- Per-process `FileDescriptorTable` translates integer fd numbers into open `FileDescriptor` objects that track inode, offset, and flags.

## Core Traits

### Inode

The central abstraction for any file, directory, device, or symlink. It is object-safe (`dyn Inode`) so the VFS can store inodes from different filesystem types in the same data structures.

```rust
pub trait Inode: Send + Sync {
    fn inode_type(&self) -> InodeType;
    fn size(&self) -> usize;
    fn permissions(&self) -> Permissions;

    fn read<'a>(&'a self, offset: usize, buf: &'a mut [u8])
        -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>>;

    fn write<'a>(&'a self, offset: usize, buf: &'a [u8])
        -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>>;

    fn lookup<'a>(&'a self, name: &'a str)
        -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>>;

    fn readdir(&self)
        -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>>;

    fn create<'a>(&'a self, name: &'a str, itype: InodeType, perms: Permissions)
        -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>>;

    fn unlink<'a>(&'a self, name: &'a str)
        -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>>;
}
```

**Key design decisions:**

- **Async I/O via boxed futures** -- Methods like `read`, `write`, and `lookup` return `Pin<Box<dyn Future>>` rather than plain values. This keeps the trait object-safe while supporting truly asynchronous operations for block-backed filesystems. In-memory implementations wrap their result in `Box::pin(async { ... })` and resolve on a single poll.
- **`poll_immediate` helper** -- For call sites that know a future will resolve instantly (e.g., VFS walking ramfs directories), `poll_immediate()` polls once with a noop waker and panics if the future returns `Pending`.
- **`try_poll_immediate` helper** -- Syscall handlers use this to attempt synchronous I/O first; if the future returns `Pending`, the handler falls back to the async `TRAP_IO` mechanism.

### FileSystem

Every mounted filesystem implements this trait:

```rust
pub trait FileSystem: Send + Sync {
    fn name(&self) -> &'static str;
    fn root(&self) -> Arc<dyn Inode>;
}
```

### FsError

All filesystem operations return `Result<T, FsError>`. The error enum maps directly to POSIX errno values via `FsError::to_errno()`:

| Variant          | errno    |
|------------------|----------|
| `NotFound`       | `ENOENT` |
| `NotADirectory`  | `ENOTDIR`|
| `IsADirectory`   | `EISDIR` |
| `AlreadyExists`  | `EEXIST` |
| `BadFd`          | `EBADF`  |
| `PermissionDenied` | `EACCES` |
| `IoError`        | `EIO`    |
| `InvalidArgument`| `EINVAL` |
| `NotSupported`   | `ENOSYS` |
| `SymlinkLoop`    | `ELOOP`  |

## VFS Mount Table

The global VFS is defined in `fs/vfs.rs` as a `SpinLock<Option<Vfs>>` static, initialized during boot and accessed through:

- `with_vfs(|vfs| ...)` -- shared access for path resolution
- `with_vfs_mut(|vfs| ...)` -- exclusive access for mounting

**Mount table structure:**

```rust
pub struct Vfs {
    mounts: BTreeMap<String, Arc<dyn FileSystem>>,
}
```

**Boot-time mount sequence:**

During `kernel_init` (in `boot.rs`), the kernel:

1. Calls `fs::vfs::init()` to create the empty VFS.
2. Discovers the ramfs `VirtualFsEntry` from the `.hadron_virtual_fs` linker section and mounts it at `/`.
3. Unpacks the bootloader-provided initrd CPIO archive into the ramfs root.
4. Creates and mounts `DevFs` at `/dev`.
5. For each discovered block device, iterates the `BlockFsEntry` entries from the `.hadron_block_fs` section.

## Path Resolution

Path resolution (`Vfs::resolve()`) works in two steps:

1. **Mount matching** -- The `path::longest_prefix_match()` function finds the longest mount-point prefix. For `/dev/null` with mounts at `/` and `/dev`, it matches `/dev`.

2. **Inode walk** -- The mount prefix is stripped, the filesystem's root inode is obtained, and the remaining components are resolved one at a time using `Inode::lookup()`. Each lookup future is evaluated with `poll_immediate()`.

**Symlink resolution:** If a path component resolves to a symlink, the VFS calls `Inode::read_link()` and recursively resolves the target. A depth counter (max 8) prevents infinite loops, returning `FsError::SymlinkLoop` if exceeded.

## File Descriptors

`fs/file.rs` defines the per-process file descriptor layer.

### OpenFlags

A bitflags type controlling how a file is opened:

```rust
pub struct OpenFlags: u32 {
    const READ     = 0b0001;
    const WRITE    = 0b0010;
    const CREATE   = 0b0100;
    const TRUNCATE = 0b1000;
}
```

### FileDescriptor

```rust
pub struct FileDescriptor {
    pub inode: Arc<dyn Inode>,
    pub offset: usize,
    pub flags: OpenFlags,
}
```

Each open file descriptor holds a reference-counted pointer to the backing inode, a byte offset, and the opening flags.

### FileDescriptorTable

```rust
pub struct FileDescriptorTable {
    fds: BTreeMap<usize, FileDescriptor>,
    next_fd: usize,
}
```

Each process owns a `FileDescriptorTable`. Key operations:

- `open(inode, flags) -> usize` -- Allocates the next fd number and inserts the descriptor.
- `insert_at(fd, inode, flags)` -- Inserts at a specific fd number. Used for stdin (0), stdout (1), stderr (2) wired to `/dev/console` during process creation.
- `close(fd)` -- Removes the fd.
- `get(fd)` / `get_mut(fd)` -- Borrows the descriptor by fd number.

## Filesystem Implementations

### RamFs

An in-memory filesystem residing entirely in the kernel heap. All operations are synchronous and resolve instantly. Used as the root filesystem and to hold the initrd contents.

**Key types:**

- `RamFs` -- Implements `FileSystem`
- `RamFsDir` -- Directory inode, holds a `BTreeMap<String, Arc<dyn Inode>>`
- `RamFsFile` -- File inode, holds a `Vec<u8>` for file content

### DevFs

Implements the `/dev` filesystem with built-in device nodes.

**Built-in nodes:**

| Node      | Type          | Behavior |
|-----------|---------------|----------|
| `null`    | `CharDevice`  | Reads return 0 bytes (EOF). Writes are silently discarded. |
| `zero`    | `CharDevice`  | Reads fill the buffer with zeros. Writes are discarded. |
| `console` | `CharDevice`  | Writes go to kernel console. Reads block for keyboard input. |
| `fb0`     | `CharDevice`  | Framebuffer memory-mappable device. |
| `ttyN`    | `CharDevice`  | Virtual terminal devices (0-5 for Alt+F1-F6). |

### TTY Layer

The `tty/` module provides virtual terminal abstractions. Each `Tty` owns a `LineDiscipline` for cooked-mode line editing and integrates with the VFS through `DevConsole` and `/dev/ttyN` inodes.

**Key operations:**

- **Cooked-mode line editing** -- Ctrl+H for backspace, Ctrl+U for line clear, Ctrl+C for signal, Enter/Ctrl+D for submit.
- **Multi-VT support** -- Alt+F1-F6 switches between 6 virtual terminals with independent process groups.
- **Signal dispatch** -- Foreground process group receives SIGINT/SIGTSTP signals.

## Block Device Abstraction

For block-device-backed filesystems, the VFS integrates with the `BlockDevice` trait:

```rust
pub trait BlockDevice: Send + Sync {
    fn read_block(&self, lba: u64, buf: &mut [u8]) -> impl Future<Output = Result<(), IoError>>;
    fn write_block(&self, lba: u64, buf: &[u8]) -> impl Future<Output = Result<(), IoError>>;
}
```

A `BlockDeviceAdapter` wraps a concrete implementation (e.g., VirtIO block, AHCI) and provides it to filesystems like FAT or ext2.

## Async I/O Integration

File I/O integrates with Hadron's cooperative async executor. When a read or write cannot proceed synchronously (e.g., awaiting a disk I/O completion), the future yields to the executor via `.await`, allowing other tasks to make progress. The executor's task mapping ensures the correct task is resumed when the I/O completes.

Synchronous contexts (early boot, kernel code that cannot yield) use `try_poll_immediate()`, which attempts a single poll of the future. If the inode operation would block, it returns `None` rather than yielding, triggering a `TRAP_IO` syscall handler for async handling.
