//! In-memory filesystem backed by heap allocations.
//!
//! `RamFs` provides a simple filesystem where all data lives on the kernel heap.
//! Used as the root filesystem and for temporary storage. All I/O completes
//! synchronously (futures resolve in a single poll).

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use hadron_core::sync::SpinLock;

use super::{DirEntry, FileSystem, FsError, Inode, InodeType, Permissions};

/// A ramfs filesystem instance.
pub struct RamFs {
    /// The root directory inode.
    root: Arc<RamInode>,
}

impl Default for RamFs {
    fn default() -> Self {
        Self::new()
    }
}

impl RamFs {
    /// Creates a new ramfs with an empty root directory.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: Arc::new(RamInode {
                itype: InodeType::Directory,
                data: SpinLock::new(Vec::new()),
                children: SpinLock::new(BTreeMap::new()),
                permissions: Permissions::all(),
            }),
        }
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &'static str {
        "ramfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}

/// A ramfs inode (file or directory).
pub struct RamInode {
    /// Inode type.
    itype: InodeType,
    /// File data (only meaningful for files).
    data: SpinLock<Vec<u8>>,
    /// Child entries (only meaningful for directories).
    children: SpinLock<BTreeMap<String, Arc<RamInode>>>,
    /// Permissions.
    permissions: Permissions,
}

impl Inode for RamInode {
    fn inode_type(&self) -> InodeType {
        self.itype
    }

    fn size(&self) -> usize {
        match self.itype {
            InodeType::File | InodeType::Symlink => self.data.lock().len(),
            _ => 0,
        }
    }

    fn permissions(&self) -> Permissions {
        self.permissions
    }

    fn read<'a>(
        &'a self,
        offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if self.itype == InodeType::Directory {
                return Err(FsError::IsADirectory);
            }
            let data = self.data.lock();
            if offset >= data.len() {
                return Ok(0);
            }
            let available = &data[offset..];
            let to_copy = buf.len().min(available.len());
            buf[..to_copy].copy_from_slice(&available[..to_copy]);
            Ok(to_copy)
        })
    }

    fn write<'a>(
        &'a self,
        offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if self.itype == InodeType::Directory {
                return Err(FsError::IsADirectory);
            }
            let mut data = self.data.lock();
            let end = offset + buf.len();
            if end > data.len() {
                data.resize(end, 0);
            }
            data[offset..end].copy_from_slice(buf);
            Ok(buf.len())
        })
    }

    fn lookup<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if self.itype != InodeType::Directory {
                return Err(FsError::NotADirectory);
            }
            let children = self.children.lock();
            children
                .get(name)
                .cloned()
                .map(|n| n as Arc<dyn Inode>)
                .ok_or(FsError::NotFound)
        })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async move {
            if self.itype != InodeType::Directory {
                return Err(FsError::NotADirectory);
            }
            let children = self.children.lock();
            Ok(children
                .iter()
                .map(|(name, inode)| DirEntry {
                    name: name.clone(),
                    inode_type: inode.itype,
                })
                .collect())
        })
    }

    fn create<'a>(
        &'a self,
        name: &'a str,
        itype: InodeType,
        perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if self.itype != InodeType::Directory {
                return Err(FsError::NotADirectory);
            }
            let mut children = self.children.lock();
            if children.contains_key(name) {
                return Err(FsError::AlreadyExists);
            }
            let new_inode = Arc::new(RamInode {
                itype,
                data: SpinLock::new(Vec::new()),
                children: SpinLock::new(BTreeMap::new()),
                permissions: perms,
            });
            children.insert(name.to_string(), new_inode.clone());
            Ok(new_inode as Arc<dyn Inode>)
        })
    }

    fn unlink<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async move {
            if self.itype != InodeType::Directory {
                return Err(FsError::NotADirectory);
            }
            let mut children = self.children.lock();
            children.remove(name).ok_or(FsError::NotFound)?;
            Ok(())
        })
    }

    fn read_link(&self) -> Result<String, FsError> {
        if self.itype != InodeType::Symlink {
            return Err(FsError::InvalidArgument);
        }
        let data = self.data.lock();
        String::from_utf8(data.clone()).map_err(|_| FsError::IoError)
    }

    fn create_symlink(
        &self,
        name: &str,
        target: &str,
        perms: Permissions,
    ) -> Result<Arc<dyn Inode>, FsError> {
        if self.itype != InodeType::Directory {
            return Err(FsError::NotADirectory);
        }
        let mut children = self.children.lock();
        if children.contains_key(name) {
            return Err(FsError::AlreadyExists);
        }
        let new_inode = Arc::new(RamInode {
            itype: InodeType::Symlink,
            data: SpinLock::new(target.as_bytes().to_vec()),
            children: SpinLock::new(BTreeMap::new()),
            permissions: perms,
        });
        children.insert(name.to_string(), new_inode.clone());
        Ok(new_inode)
    }
}
