# Phase 8: Async VFS & Ramfs

## Goal

Create a Virtual Filesystem layer with async inode operations, an in-memory ramfs, an initramfs CPIO unpacker, and special filesystems (devfs, procfs). After this phase, the kernel has a unified async file abstraction where heap-backed operations resolve immediately and block-backed operations can await I/O completion through the same interface.

## Key Design: Async Inode Trait

The central abstraction is the `Inode` trait with async `read` and `write` methods:

```rust
pub trait Inode: Send + Sync {
    fn inode_type(&self) -> InodeType;
    fn size(&self) -> usize;
    fn permissions(&self) -> Permissions;

    /// Read data from this inode at the given offset.
    fn read<'a>(&'a self, offset: usize, buf: &'a mut [u8])
        -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>>;

    /// Write data to this inode at the given offset.
    fn write<'a>(&'a self, offset: usize, buf: &'a [u8])
        -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>>;

    /// Look up a child by name (directories only).
    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError>;

    /// List directory entries.
    fn readdir(&self) -> Result<Vec<DirEntry>, FsError>;

    /// Create a new child inode (directories only).
    fn create(&self, name: &str, itype: InodeType, perms: Permissions)
        -> Result<Arc<dyn Inode>, FsError>;

    /// Remove a child by name.
    fn unlink(&self, name: &str) -> Result<(), FsError>;
}
```

For ramfs (heap-backed), the returned futures resolve immediately -- the `async` wrapper contains no `.await` points. For a block-backed filesystem such as ext2 (Phase 12), the futures await I/O completion from the block device. This provides a unified interface without penalizing in-memory operations.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `hadron-kernel/src/fs/mod.rs` | Module root, `Inode` trait, `FsError`, filesystem registration |
| `hadron-kernel/src/fs/vfs.rs` | VFS core: mount table, path resolution |
| `hadron-kernel/src/fs/file.rs` | `FileDescriptor`, `FileDescriptorTable` |
| `hadron-kernel/src/fs/path.rs` | Path parsing and canonicalization |
| `hadron-kernel/src/fs/mount.rs` | Mount points and mount operations |
| `hadron-kernel/src/fs/ramfs.rs` | Heap-backed in-memory filesystem |
| `hadron-kernel/src/fs/initramfs.rs` | CPIO newc format unpacker |
| `hadron-kernel/src/fs/devfs.rs` | `/dev/console`, `/dev/null`, `/dev/zero` |
| `hadron-kernel/src/fs/procfs.rs` | `/proc/meminfo` |

## Key Data Structures

### VFS Mount Table

```rust
pub struct Vfs {
    mounts: BTreeMap<String, Arc<dyn FileSystem>>,
}

impl Vfs {
    pub fn mount(&mut self, path: &str, fs: Arc<dyn FileSystem>) -> Result<(), FsError> {
        self.mounts.insert(path.to_string(), fs);
        Ok(())
    }

    /// Resolve a path to an inode, following mount points.
    pub fn resolve(&self, path: &str) -> Result<Arc<dyn Inode>, FsError> {
        // Find longest matching mount point prefix
        // Walk remaining path components via inode.lookup()
    }
}
```

### File Descriptor Table

```rust
pub struct FileDescriptor {
    pub inode: Arc<dyn Inode>,
    pub offset: usize,
    pub flags: OpenFlags,
}

pub struct FileDescriptorTable {
    fds: BTreeMap<usize, FileDescriptor>,
    next_fd: usize,
}

impl FileDescriptorTable {
    pub fn open(&mut self, inode: Arc<dyn Inode>, flags: OpenFlags) -> usize {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.fds.insert(fd, FileDescriptor { inode, offset: 0, flags });
        fd
    }

    pub fn close(&mut self, fd: usize) -> Result<(), FsError> {
        self.fds.remove(&fd).ok_or(FsError::BadFd)?;
        Ok(())
    }

    pub fn get(&self, fd: usize) -> Result<&FileDescriptor, FsError> {
        self.fds.get(&fd).ok_or(FsError::BadFd)
    }

    pub fn get_mut(&mut self, fd: usize) -> Result<&mut FileDescriptor, FsError> {
        self.fds.get_mut(&fd).ok_or(FsError::BadFd)
    }
}
```

### Ramfs

```rust
pub struct RamFs {
    root: Arc<RamInode>,
}

struct RamInode {
    itype: InodeType,
    data: Mutex<Vec<u8>>,
    children: Mutex<BTreeMap<String, Arc<RamInode>>>,
    permissions: Permissions,
}

impl Inode for RamInode {
    fn read<'a>(&'a self, offset: usize, buf: &'a mut [u8])
        -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>>
    {
        Box::pin(async move {
            let data = self.data.lock();
            let available = data.len().saturating_sub(offset);
            let to_read = buf.len().min(available);
            buf[..to_read].copy_from_slice(&data[offset..offset + to_read]);
            Ok(to_read)
        })
    }
    // write() follows the same pattern -- future resolves immediately.
}
```

The `Box::pin(async move { ... })` wrapper contains no `.await`, so it completes in a single poll. This is the expected pattern for in-memory filesystems.

### Initramfs (CPIO newc)

The initramfs unpacker parses a CPIO newc archive from a boot module and populates the root ramfs:

- Reads the fixed 110-byte header per entry.
- Extracts filename and file data.
- Creates directory tree and files via `Inode::create()` and `Inode::write()`.
- Stops at the `TRAILER!!!` sentinel entry.

### devfs

Provides device nodes mounted at `/dev`:

- `/dev/console` -- reads from and writes to the serial port.
- `/dev/null` -- discards writes, reads return 0 bytes.
- `/dev/zero` -- reads return zero-filled buffers, writes are discarded.

Each device node implements the `Inode` trait with async read/write.

### procfs

Provides kernel information mounted at `/proc`:

- `/proc/meminfo` -- reports total and free memory from the physical memory allocator.

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| All VFS code | Service | Pure data structure management |
| Ramfs | Service | Heap-backed, no hardware interaction |
| Initramfs unpacker | Service | Parses byte array from boot module |
| devfs, procfs | Service | Virtual filesystems, safe code |
| File descriptor table | Service | Per-process bookkeeping |

This phase is **entirely** service code -- no new unsafe frame code is needed.

## Dependencies

- **Phase 4**: Kernel heap (for `Vec`, `BTreeMap`, `Arc`, `Box::pin`).
- **Phase 7**: Syscall interface (for file descriptor operations: `sys_read`, `sys_write`, `sys_open`, `sys_close`).

## Milestone

Kernel mounts ramfs at `/`, unpacks initramfs, and demonstrates file operations:

```
VFS: Mounted ramfs at /
Initramfs: Unpacked 5 files
ls /: bin dev proc
cat /bin/hello: Hello from initramfs!
/dev/null: write 100 bytes -> 100, read -> 0
/proc/meminfo: MemTotal: 256 MiB, MemFree: 240 MiB
```
