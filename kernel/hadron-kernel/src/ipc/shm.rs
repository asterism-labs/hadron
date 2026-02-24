//! Shared memory objects for IPC.
//!
//! A [`ShmObject`] owns a set of physical frames that can be mapped into
//! multiple process address spaces simultaneously. The compositor uses this
//! to share pixel buffers with client processes.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};
use crate::mm::PAGE_SIZE;
use crate::mm::pmm;
use crate::paging::{PhysFrame, Size4KiB};
use crate::addr::PhysAddr;

/// A shared memory object backed by physical frames.
///
/// Frames are allocated from the PMM and zeroed on creation. Multiple
/// processes can map the same object. Frames are returned to the PMM
/// when the last reference is dropped.
pub struct ShmObject {
    /// Physical frames backing this shared memory region.
    frames: Vec<PhysFrame<Size4KiB>>,
    /// Logical size in bytes (may not be page-aligned; frames cover the
    /// page-aligned size).
    size: usize,
}

impl ShmObject {
    /// Allocate a new shared memory object of the given size.
    ///
    /// `size` is rounded up to page alignment. All pages are zeroed.
    /// Returns `None` if the PMM cannot satisfy the allocation.
    pub fn new(size: usize) -> Option<Arc<Self>> {
        if size == 0 {
            return None;
        }

        let aligned = (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let page_count = aligned / PAGE_SIZE;
        let hhdm_offset = crate::mm::hhdm::offset();

        let frames = pmm::with(|pmm| {
            let mut allocated = Vec::with_capacity(page_count);
            for _ in 0..page_count {
                let frame = pmm.allocate_frame()?;

                // Zero the frame via HHDM.
                let ptr = (hhdm_offset + frame.start_address().as_u64()).as_mut_ptr::<u8>();
                // SAFETY: Frame was just allocated; zeroing via HHDM is safe.
                unsafe {
                    core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
                }

                allocated.push(frame);
            }
            Some(allocated)
        })?;

        Some(Arc::new(Self { frames, size }))
    }

    /// Returns the logical size in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.size
    }
}

impl Drop for ShmObject {
    fn drop(&mut self) {
        pmm::with(|pmm| {
            for &frame in &self.frames {
                // SAFETY: These frames were allocated by `ShmObject::new` and are
                // no longer mapped by any process (all Arc refs are gone).
                let _ = unsafe { pmm.deallocate_frame(frame) };
            }
        });
    }
}

impl Inode for ShmObject {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        self.size
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
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

    fn shared_phys_frames(&self) -> Result<Vec<PhysAddr>, FsError> {
        Ok(self
            .frames
            .iter()
            .map(|f| f.start_address())
            .collect())
    }
}
