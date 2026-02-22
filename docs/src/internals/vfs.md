# Virtual Filesystem

The Hadron kernel provides a Virtual Filesystem (VFS) layer that abstracts over
different filesystem implementations behind a uniform set of traits. All file
I/O -- whether targeting an in-memory ramfs, a device node in devfs, or a
block-device-backed FAT or ISO 9660 volume -- passes through the VFS.

The implementation lives in `kernel/hadron-kernel/src/fs/`, with filesystem
drivers in `kernel/hadron-drivers/src/fs/`.

## Architecture overview

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
```

The VFS has three logical layers:

1. **Core traits** (`fs/mod.rs`) -- `Inode` and `FileSystem` define the
   interface every filesystem must implement.
2. **Mount table and path resolution** (`fs/vfs.rs`) -- A global `Vfs` struct
   maps mount paths to `FileSystem` instances and resolves absolute paths to
   inodes.
3. **File descriptors** (`fs/file.rs`) -- Per-process `FileDescriptorTable`
   translates integer fd numbers into open `FileDescriptor` objects that
   track inode, offset, and flags.

## Core traits

### `Inode`

Defined in `fs/mod.rs`, `Inode` is the central abstraction for any file,
directory, device, or symlink. It is object-safe (`dyn Inode`) so the VFS
can store inodes from different filesystem types in the same data structures.

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

    fn read_link(&self) -> Result<String, FsError> { ... }
    fn create_symlink(&self, name: &str, target: &str, perms: Permissions)
        -> Result<Arc<dyn Inode>, FsError> { ... }
}
```

Key design decisions:

- **Async I/O via boxed futures.** Methods like `read`, `write`, and `lookup`
  return `Pin<Box<dyn Future>>` rather than plain values. This keeps the trait
  object-safe while supporting truly asynchronous operations for block-backed
  filesystems. In-memory implementations (ramfs, devfs) wrap their result in
  `Box::pin(async { ... })` and resolve on a single poll.
- **`poll_immediate` helper.** For call sites that know a future will resolve
  instantly (e.g., the VFS walking ramfs directories), `poll_immediate()`
  polls once with a noop waker and panics if the future returns `Pending`.
- **`try_poll_immediate` helper.** Syscall handlers use this to attempt
  synchronous I/O first; if the future returns `Pending` the handler falls
  back to the async `TRAP_IO` mechanism.

### `InodeType`

```rust
pub enum InodeType {
    File,
    Directory,
    CharDevice,
    Symlink,
}
```

### `FileSystem`

Every mounted filesystem implements this trait:

```rust
pub trait FileSystem: Send + Sync {
    fn name(&self) -> &'static str;
    fn root(&self) -> Arc<dyn Inode>;
}
```

`name()` returns a human-readable identifier (e.g., `"ramfs"`, `"devfs"`,
`"fat"`, `"iso9660"`). `root()` returns the root directory inode from which
all path resolution begins after the mount prefix is stripped.

### `FsError`

All filesystem operations return `Result<T, FsError>`. The error enum maps
directly to POSIX errno values via `FsError::to_errno()`:

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

## VFS mount table

The global VFS is defined in `fs/vfs.rs` as a `SpinLock<Option<Vfs>>` static.
It is initialized during boot by `vfs::init()` and accessed through two
accessor functions:

- `with_vfs(|vfs| ...)` -- shared access for path resolution
- `with_vfs_mut(|vfs| ...)` -- exclusive access for mounting

### `Vfs` struct

```rust
pub struct Vfs {
    mounts: BTreeMap<String, Arc<dyn FileSystem>>,
}
```

The mount table is a `BTreeMap` keyed by mount-point path strings. Mounting
inserts into this map:

```rust
vfs.mount("/", ramfs);        // root filesystem
vfs.mount("/dev", devfs);     // device filesystem
vfs.mount("/mnt", fat_fs);    // block-backed FS
vfs.mount("/cdrom", iso_fs);  // CD-ROM
```

### Boot-time mount sequence

During `kernel_init` (in `boot.rs`), the kernel:

1. Calls `fs::vfs::init()` to create the empty VFS.
2. Discovers the ramfs `VirtualFsEntry` from the `.hadron_virtual_fs` linker
   section and mounts it at `/`.
3. Unpacks the bootloader-provided initrd CPIO archive into the ramfs root
   using an `InitramFsEntry` from the `.hadron_initramfs` section.
4. Creates and mounts `DevFs` at `/dev`.
5. For each discovered block device (e.g., `virtio-blk-0`, `ahci-0`), iterates
   the `BlockFsEntry` entries from the `.hadron_block_fs` section, calling
   each `mount` function until one succeeds.

## Path resolution

Path resolution is implemented by `Vfs::resolve()` in `fs/vfs.rs`. It works in
two phases:

1. **Mount matching.** The path utilities in `fs/path.rs` find the
   longest mount-point prefix. For example, given mounts at `/` and `/dev`,
   the path `/dev/null` matches `/dev`. The function
   `path::longest_prefix_match()` compares path prefixes correctly
   (i.e., `/dev` matches `/dev/null` but not `/device`).

2. **Inode walk.** The mount prefix is stripped (via
   `path::strip_mount_prefix()`), the filesystem's root inode is obtained,
   and the remaining components are resolved one at a time using
   `Inode::lookup()`. Each lookup future is evaluated with `poll_immediate()`.

### Symlink resolution

If a path component resolves to an `InodeType::Symlink`, the VFS calls
`Inode::read_link()` to obtain the target path and recursively resolves it.
A depth counter prevents infinite loops, capped at `MAX_SYMLINK_DEPTH = 8`.
Exceeding this returns `FsError::SymlinkLoop`.

### Path utilities (`fs/path.rs`)

| Function                | Purpose |
|-------------------------|---------|
| `components(path)`      | Splits on `/`, filters empty segments |
| `is_absolute(path)`     | Returns `true` if the path starts with `/` |
| `longest_prefix_match(path, mounts)` | Finds the longest matching mount point |
| `strip_mount_prefix(path, mount)` | Removes the mount prefix, returning the remainder |

## File descriptors

`fs/file.rs` defines the per-process file descriptor layer.

### `OpenFlags`

A bitflags type controlling how a file is opened:

```rust
pub struct OpenFlags: u32 {
    const READ     = 0b0001;
    const WRITE    = 0b0010;
    const CREATE   = 0b0100;
    const TRUNCATE = 0b1000;
}
```

### `FileDescriptor`

```rust
pub struct FileDescriptor {
    pub inode: Arc<dyn Inode>,
    pub offset: usize,
    pub flags: OpenFlags,
}
```

Each open file descriptor holds a reference-counted pointer to the backing
inode, a byte offset tracking the current read/write position, and the flags
it was opened with.

### `FileDescriptorTable`

```rust
pub struct FileDescriptorTable {
    fds: BTreeMap<usize, FileDescriptor>,
    next_fd: usize,
}
```

Each process owns a `FileDescriptorTable`. Key operations:

- `open(inode, flags) -> usize` -- allocates the next fd number, inserts
  the descriptor, and returns the fd.
- `insert_at(fd, inode, flags)` -- inserts at a specific fd number. Used to
  set up the standard file descriptors: stdin (0), stdout (1), stderr (2)
  are wired to `/dev/console` during process creation.
- `close(fd) -> Result<(), FsError>` -- removes the fd, returning
  `FsError::BadFd` if it does not exist.
- `get(fd)` / `get_mut(fd)` -- borrow the descriptor by fd number.

## Devfs

`fs/devfs.rs` implements the `/dev` filesystem with three built-in device
nodes. The filesystem is entirely in-memory; all operations resolve
immediately.

### `DevFs` and `DevFsDir`

`DevFs` implements `FileSystem`. Its root inode is a `DevFsDir` that holds a
static `BTreeMap<&'static str, Arc<dyn Inode>>` with the following entries:

| Node      | Type          | Behavior |
|-----------|---------------|----------|
| `null`    | `CharDevice`  | Reads return 0 bytes (EOF). Writes are silently discarded. |
| `zero`    | `CharDevice`  | Reads fill the buffer with zeros. Writes are discarded. |
| `console` | `CharDevice`  | Writes go to kernel console output. Reads block for keyboard input. |

The devfs directory is read-only: `create` and `unlink` return
`FsError::NotSupported`.

### `/dev/console` and `ConsoleReadFuture`

`DevConsole` is the only device node whose reads can genuinely block. Its
`Inode::read()` returns a `ConsoleReadFuture` that implements the
check-register-recheck pattern to avoid races between data arrival and waker
registration:

1. Poll keyboard hardware and check the ready buffer.
2. If data is available, return `Poll::Ready`.
3. Register the waker with the `INPUT_READY` wait queue.
4. Re-check the buffer (catches IRQs that fired between steps 1 and 3).
5. If still no data, return `Poll::Pending`.

Writes interpret the buffer as UTF-8 (with a byte-by-byte fallback) and send
it to the kernel's `kprint!` macro.

## TTY subsystem

The `tty/` module provides virtual terminal abstractions. Each `Tty` owns a
`LineDiscipline` for cooked-mode line editing, a per-VT foreground process
group, and a waker slot for async reader notification. `/dev/console` delegates
reads to the active TTY; `/dev/ttyN` nodes map directly to specific VTs.

### Data flow

```
PS/2 hardware (ports 0x60, 0x64)
        |
   IRQ1 handler  ->  SCANCODE_BUF (ring buffer, IrqSpinLock)
        |                  |
   TTY_WAKER.wake()        |
                           v
            Tty::poll_hardware()
                   |
       LineDiscipline::process_scancode()   (decode, echo, line editing)
                   |
          LineDiscipline::ready_buf         (completed lines)
                   |
           Tty::try_read(buf)               (non-blocking read for TtyReadFuture)
```

### Key components

- **`SCANCODE_BUF`**: An `IrqSpinLock<RingBuf<u8, 64>>` filled by the
  keyboard IRQ handler. Scancodes are buffered here rather than processed in
  IRQ context to avoid taking the logger lock (needed for character echo)
  from interrupt context.

- **`LineDiscipline`** (`tty/ldisc.rs`): Contains the `ready_buf` (completed
  lines available for reading), a `line_buf` for the line currently being
  edited, modifier state, and the `extended_prefix` flag for 0xE0-prefixed
  scancodes. Handles Ctrl+C (SIGINT), Ctrl+D (EOF), backspace, and Enter.

- **`Tty`** (`tty/mod.rs`): Wraps a `LineDiscipline` behind an `IrqSpinLock`,
  manages the per-VT foreground PGID, and handles IRQ-to-waker routing.

- **`DevTty`** (`tty/device.rs`): VFS `Inode` implementation for `/dev/ttyN`,
  using `TtyReadFuture` with a two-phase waker strategy.

- **`tty::init()`**: Registers the IRQ1 handler via the interrupt dispatch
  table and unmasks IRQ1 in the I/O APIC.

## Block adapter

`fs/block_adapter.rs` bridges the gap between async sector-oriented block
devices and the synchronous byte-oriented `hadris_io::Read + Seek + Write`
traits used by filesystem implementations like `hadris-fat` and `hadris-iso`.

### `BlockDeviceAdapter<D: BlockDevice>`

```rust
pub struct BlockDeviceAdapter<D: BlockDevice> {
    device: D,
    position: u64,
    sector_buf: Vec<u8>,
    total_size: u64,
}
```

The adapter maintains a byte-level cursor (`position`) and a heap-allocated
scratch buffer (`sector_buf`) sized to one sector. Each I/O call processes at
most one sector's worth of data; the `read_exact` / `write_all` default
methods in `hadris_io` loop as needed.

**Read path:** Computes the sector number and offset within the sector from
`position`, calls `block_on(device.read_sector(...))` to synchronously
execute the async read, then copies the relevant bytes into the caller's
buffer.

**Write path:** Uses a read-modify-write pattern. The target sector is read
into `sector_buf`, the new data is overlaid at the correct offset, and the
modified sector is written back.

**Seek:** Supports `SeekFrom::Start`, `SeekFrom::End`, and
`SeekFrom::Current`, clamping against the total device size.

### Type-erased variant

The type alias `BoxedBlockAdapter` specializes the adapter over
`Box<dyn DynBlockDevice>`, which is itself a dyn-safe wrapper around any
concrete `BlockDevice`. This allows filesystem `mount` functions in
`BlockFsEntry` to receive block devices without generic parameters:

```
Concrete driver (e.g., AhciDisk)
    -> DynBlockDeviceWrapper<AhciDisk>       (implements DynBlockDevice)
    -> Box<dyn DynBlockDevice>               (type-erased)
    -> BlockDeviceAdapter<Box<dyn DynBlockDevice>>  (implements hadris_io traits)
    -> Filesystem mount function (reads sectors via hadris_io)
```

## Filesystem registration

Filesystem drivers register themselves via linker-section macros defined in
`driver_api/registration.rs`. Three entry types exist:

| Entry type        | Linker section        | Purpose |
|-------------------|-----------------------|---------|
| `BlockFsEntry`    | `.hadron_block_fs`    | Mount function for block-device-backed filesystems (FAT, ISO 9660) |
| `VirtualFsEntry`  | `.hadron_virtual_fs`  | Factory function for memory-backed filesystems (ramfs) |
| `InitramFsEntry`  | `.hadron_initramfs`   | Unpacker for initrd archives (CPIO) |

Driver crates use the corresponding macros to register entries:

```rust
// In hadron-drivers/src/fs/ramfs.rs:
hadron_kernel::virtual_fs_entry!(RAMFS_ENTRY, VirtualFsEntry {
    name: "ramfs",
    create: create_ramfs,
});

// In hadron-drivers/src/fs/fat.rs:
hadron_kernel::block_fs_entry!(FAT_FS_ENTRY, BlockFsEntry {
    name: "fat",
    mount: mount_fat,
});
```

The kernel iterates these linker-section entries at boot without requiring a
runtime registry, keeping filesystem discovery zero-overhead and statically
determined at link time.
