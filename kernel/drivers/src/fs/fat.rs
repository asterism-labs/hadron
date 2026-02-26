//! FAT12/16/32 filesystem driver.
//!
//! Mounts FAT volumes from block devices using the `hadris-fat` crate.
//! Directory navigation and file reads are bridged from async block device
//! I/O to synchronous `hadris_io` calls via [`BlockDeviceAdapter`]. Write
//! support (create, unlink, write) is not yet implemented.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use hadris_fat::{DirectoryEntry, FatFs, FatFsReadExt, FileEntry};
use hadron_kernel::fs::block_adapter::BoxedBlockAdapter;

use hadron_kernel::fs::{DirEntry, FileSystem, FsError, Inode, InodeType, Permissions};

/// Wrapper providing [`Sync`] for [`FatFs`].
///
/// `FatFs` contains `Cell` fields in `Fat32FsExt` (`free_count`, `next_free`)
/// that are `!Sync`. These are FSInfo cache values only modified during write
/// operations, and all data I/O is already serialized through `spin::Mutex`.
struct SharedFatFs(FatFs<BoxedBlockAdapter>);

// SAFETY: FatFs uses spin::Mutex for all data I/O operations. The !Sync Cell
// fields are FSInfo free-cluster hints, only updated during formatted writes
// which are serialized by the same mutex.
unsafe impl Sync for SharedFatFs {}

/// FAT12/16/32 filesystem backed by a block device.
pub struct FatFileSystem {
    /// Shared filesystem state, wrapped for `Sync` safety.
    inner: Arc<SharedFatFs>,
}

impl FatFileSystem {
    /// Mount a FAT filesystem from the given block device adapter.
    ///
    /// Automatically detects FAT12, FAT16, or FAT32 from the boot sector.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::IoError`] if the volume cannot be parsed.
    pub fn mount(adapter: BoxedBlockAdapter) -> Result<Self, FsError> {
        let fs = FatFs::open(adapter).map_err(|_| FsError::IoError)?;
        Ok(Self {
            inner: Arc::new(SharedFatFs(fs)),
        })
    }
}

impl FileSystem for FatFileSystem {
    fn name(&self) -> &'static str {
        "fat"
    }

    fn root(&self) -> Arc<dyn Inode> {
        Arc::new(FatDirInode {
            fs: self.inner.clone(),
            kind: FatDirKind::Root,
        })
    }
}

/// Whether this directory inode is the root or a subdirectory.
enum FatDirKind {
    /// Root directory (uses `FatFs::root_dir()`).
    Root,
    /// Subdirectory (opened via `FatFs::open_dir_entry()`).
    Subdirectory(FileEntry),
}

/// Directory inode for FAT filesystems.
struct FatDirInode {
    /// Shared reference to the FAT filesystem.
    fs: Arc<SharedFatFs>,
    /// How to open this directory.
    kind: FatDirKind,
}

// SAFETY: SharedFatFs is Sync (via unsafe impl above). FileEntry contains
// only owned data types (ShortFileName, Option<LongFileName>, bitflags, usize,
// Cluster<usize>) which are all Send + Sync.
unsafe impl Send for FatDirInode {}
unsafe impl Sync for FatDirInode {}

impl Inode for FatDirInode {
    fn inode_type(&self) -> InodeType {
        InodeType::Directory
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn lookup<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async move {
            let dir = match &self.kind {
                FatDirKind::Root => self.fs.0.root_dir(),
                FatDirKind::Subdirectory(entry) => self
                    .fs
                    .0
                    .open_dir_entry(entry)
                    .map_err(|_| FsError::IoError)?,
            };
            let entry = dir
                .find(name)
                .map_err(|_| FsError::IoError)?
                .ok_or(FsError::NotFound)?;

            if entry.is_directory() {
                Ok(Arc::new(FatDirInode {
                    fs: self.fs.clone(),
                    kind: FatDirKind::Subdirectory(entry),
                }) as Arc<dyn Inode>)
            } else {
                Ok(Arc::new(FatFileInode {
                    fs: self.fs.clone(),
                    entry,
                }) as Arc<dyn Inode>)
            }
        })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async move {
            let dir = match &self.kind {
                FatDirKind::Root => self.fs.0.root_dir(),
                FatDirKind::Subdirectory(entry) => self
                    .fs
                    .0
                    .open_dir_entry(entry)
                    .map_err(|_| FsError::IoError)?,
            };
            let mut entries = Vec::new();
            for entry_result in dir.entries() {
                let DirectoryEntry::Entry(file_entry) =
                    entry_result.map_err(|_| FsError::IoError)?;
                let name_str = file_entry.name();
                // Skip current and parent directory entries.
                if name_str == "." || name_str == ".." {
                    continue;
                }
                let inode_type = if file_entry.is_directory() {
                    InodeType::Directory
                } else {
                    InodeType::File
                };
                entries.push(DirEntry {
                    name: String::from(name_str),
                    inode_type,
                });
            }
            Ok(entries)
        })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }
}

/// File inode for FAT filesystems.
struct FatFileInode {
    /// Shared reference to the FAT filesystem.
    fs: Arc<SharedFatFs>,
    /// The file's directory entry metadata (cluster, size, name, etc.).
    entry: FileEntry,
}

// SAFETY: Same reasoning as FatDirInode.
unsafe impl Send for FatFileInode {}
unsafe impl Sync for FatFileInode {}

/// Size of the temporary buffer used when skipping bytes for offset reads.
const SKIP_BUF_SIZE: usize = 512;

impl Inode for FatFileInode {
    fn inode_type(&self) -> InodeType {
        InodeType::File
    }

    fn size(&self) -> usize {
        self.entry.size()
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn read<'a>(
        &'a self,
        offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            let file_size = self.entry.size();
            if offset >= file_size {
                return Ok(0);
            }

            let mut reader = self
                .fs
                .0
                .read_file(&self.entry)
                .map_err(|_| FsError::IoError)?;

            // Skip `offset` bytes by reading and discarding.
            let mut remaining = offset;
            let mut skip_buf = [0u8; SKIP_BUF_SIZE];
            while remaining > 0 {
                let to_skip = remaining.min(SKIP_BUF_SIZE);
                let n = reader
                    .read(&mut skip_buf[..to_skip])
                    .map_err(|_| FsError::IoError)?;
                if n == 0 {
                    return Ok(0);
                }
                remaining -= n;
            }

            // Read the actual data.
            let to_read = buf.len().min(file_size - offset);
            let mut total = 0;
            while total < to_read {
                let n = reader
                    .read(&mut buf[total..to_read])
                    .map_err(|_| FsError::IoError)?;
                if n == 0 {
                    break;
                }
                total += n;
            }
            Ok(total)
        })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn lookup<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

#[cfg(target_os = "none")]
fn fat_mount(
    disk: alloc::boxed::Box<dyn hadron_kernel::driver_api::dyn_dispatch::DynBlockDevice>,
) -> Result<alloc::sync::Arc<dyn hadron_kernel::fs::FileSystem>, hadron_kernel::fs::FsError> {
    let adapter = hadron_kernel::fs::block_adapter::BlockDeviceAdapter::new(disk);
    let fs = FatFileSystem::mount(adapter)?;
    Ok(alloc::sync::Arc::new(fs))
}

#[cfg(target_os = "none")]
hadron_kernel::block_fs_entry!(
    FAT_FS_ENTRY,
    hadron_kernel::driver_api::registration::BlockFsEntry {
        name: "fat",
        mount: fat_mount,
    }
);
