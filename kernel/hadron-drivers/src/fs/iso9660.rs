//! ISO 9660 filesystem driver (read-only).
//!
//! Mounts ISO 9660 images from block devices using the `hadris-iso` crate.
//! Directory navigation and file reads are bridged from async block device
//! I/O to synchronous `hadris_io` calls via [`BlockDeviceAdapter`].

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use hadris_iso::directory::DirectoryRef;
use hadris_iso::read::IsoImage;
use hadron_kernel::fs::block_adapter::BoxedBlockAdapter;

use hadron_kernel::fs::{DirEntry, FileSystem, FsError, Inode, InodeType, Permissions};

/// ISO 9660 sector size in bytes.
const ISO_SECTOR_SIZE: u64 = 2048;

/// ISO 9660 filesystem backed by a block device.
pub struct Iso9660Fs {
    /// The parsed ISO image.
    image: Arc<IsoImage<BoxedBlockAdapter>>,
}

impl Iso9660Fs {
    /// Mount an ISO 9660 image from the given block device adapter.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::IoError`] if the image cannot be parsed.
    pub fn mount(adapter: BoxedBlockAdapter) -> Result<Self, FsError> {
        let image = IsoImage::open(adapter).map_err(|_| FsError::IoError)?;
        Ok(Self {
            image: Arc::new(image),
        })
    }
}

impl FileSystem for Iso9660Fs {
    fn name(&self) -> &'static str {
        "iso9660"
    }

    fn root(&self) -> Arc<dyn Inode> {
        let root = self.image.root_dir();
        Arc::new(Iso9660DirInode {
            image: self.image.clone(),
            dir_ref: root.dir_ref(),
        })
    }
}

/// Directory inode for ISO 9660.
struct Iso9660DirInode {
    /// Reference to the ISO image for I/O operations.
    image: Arc<IsoImage<BoxedBlockAdapter>>,
    /// Location and size of this directory's data on disk.
    dir_ref: DirectoryRef,
}

// SAFETY: All interior data is protected by spin::Mutex inside IsoImage,
// and BlockDevice requires Send + Sync.
unsafe impl Send for Iso9660DirInode {}
unsafe impl Sync for Iso9660DirInode {}

impl Inode for Iso9660DirInode {
    fn inode_type(&self) -> InodeType {
        InodeType::Directory
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
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
            let dir = self.image.open_dir(self.dir_ref);
            for entry_result in dir.entries() {
                let entry = entry_result.map_err(|_| FsError::IoError)?;
                if entry.is_special() {
                    continue;
                }
                let entry_name = entry.display_name();
                if entry_name.as_ref() == name {
                    return if entry.is_directory() {
                        let child_ref = entry
                            .as_dir_ref(&*self.image)
                            .map_err(|_| FsError::IoError)?;
                        Ok(Arc::new(Iso9660DirInode {
                            image: self.image.clone(),
                            dir_ref: child_ref,
                        }) as Arc<dyn Inode>)
                    } else {
                        let header = entry.header();
                        Ok(Arc::new(Iso9660FileInode {
                            image: self.image.clone(),
                            extent_lba: u64::from(header.extent.read()),
                            file_size: header.data_len.read() as usize,
                        }) as Arc<dyn Inode>)
                    };
                }
            }
            Err(FsError::NotFound)
        })
    }

    fn readdir(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async move {
            let dir = self.image.open_dir(self.dir_ref);
            let mut entries = Vec::new();
            for entry_result in dir.entries() {
                let entry = entry_result.map_err(|_| FsError::IoError)?;
                if entry.is_special() {
                    continue;
                }
                let name = entry.display_name().into_owned();
                let inode_type = if entry.is_directory() {
                    InodeType::Directory
                } else {
                    InodeType::File
                };
                entries.push(DirEntry { name, inode_type });
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

/// File inode for ISO 9660.
struct Iso9660FileInode {
    /// Reference to the ISO image for I/O operations.
    image: Arc<IsoImage<BoxedBlockAdapter>>,
    /// Starting logical block address of the file data.
    extent_lba: u64,
    /// File size in bytes.
    file_size: usize,
}

// SAFETY: Same as Iso9660DirInode — interior mutex + BlockDevice bounds.
unsafe impl Send for Iso9660FileInode {}
unsafe impl Sync for Iso9660FileInode {}

impl Inode for Iso9660FileInode {
    fn inode_type(&self) -> InodeType {
        InodeType::File
    }

    fn size(&self) -> usize {
        self.file_size
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if offset >= self.file_size {
                return Ok(0);
            }
            let remaining = self.file_size - offset;
            let to_read = buf.len().min(remaining);
            let byte_offset = self.extent_lba * ISO_SECTOR_SIZE + offset as u64;
            self.image
                .read_bytes_at(byte_offset, &mut buf[..to_read])
                .map_err(|_| FsError::IoError)?;
            Ok(to_read)
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

    fn readdir(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
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
fn iso9660_mount(
    disk: alloc::boxed::Box<dyn hadron_kernel::driver_api::dyn_dispatch::DynBlockDevice>,
) -> Result<alloc::sync::Arc<dyn hadron_kernel::fs::FileSystem>, hadron_kernel::fs::FsError> {
    let adapter = hadron_kernel::fs::block_adapter::BlockDeviceAdapter::new(disk);
    let fs = Iso9660Fs::mount(adapter)?;
    Ok(alloc::sync::Arc::new(fs))
}

#[cfg(target_os = "none")]
hadron_kernel::block_fs_entry!(
    ISO9660_FS_ENTRY,
    hadron_kernel::driver_api::registration::BlockFsEntry {
        name: "iso9660",
        mount: iso9660_mount,
    }
);
