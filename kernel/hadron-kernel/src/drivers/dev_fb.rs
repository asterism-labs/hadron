//! `/dev/fb0` device inode.
//!
//! Exposes the framebuffer to userspace via ioctl (for querying dimensions)
//! and mmap (for direct pixel access). Read/write are not supported — all
//! pixel access goes through the memory-mapped buffer.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use crate::addr::PhysAddr;
use crate::driver_api::framebuffer::{Framebuffer, PixelFormat};
use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};

/// Framebuffer device inode wrapping an `Arc<dyn Framebuffer>`.
pub struct DevFramebuffer {
    fb: Arc<dyn Framebuffer>,
}

impl DevFramebuffer {
    /// Creates a new framebuffer device inode.
    pub fn new(fb: Arc<dyn Framebuffer>) -> Self {
        Self { fb }
    }
}

impl Inode for DevFramebuffer {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
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

    fn ioctl(&self, cmd: u32, arg: usize) -> Result<usize, FsError> {
        match cmd {
            hadron_syscall::FBIOGET_INFO => {
                let info = self.fb.info();
                let fb_info = hadron_syscall::FbInfo {
                    width: info.width,
                    height: info.height,
                    pitch: info.pitch,
                    bpp: u32::from(info.bpp),
                    pixel_format: match info.pixel_format {
                        PixelFormat::Rgb32 => 0,
                        PixelFormat::Bgr32 => 1,
                        PixelFormat::Bitmask { .. } => 1, // treat as BGR32 fallback
                    },
                };

                let ptr = arg as *mut hadron_syscall::FbInfo;
                if ptr.is_null() {
                    return Err(FsError::InvalidArgument);
                }
                // SAFETY: The caller (ioctl syscall handler) has validated the
                // user pointer and switched to the user's address space (CR3).
                unsafe {
                    core::ptr::write_volatile(ptr, fb_info);
                }
                Ok(0)
            }
            _ => Err(FsError::InvalidArgument),
        }
    }

    fn mmap_phys(&self) -> Result<(PhysAddr, usize), FsError> {
        let info = self.fb.info();
        let phys = self.fb.physical_base();
        let size = info.pitch as usize * info.height as usize;
        Ok((phys, size))
    }
}
