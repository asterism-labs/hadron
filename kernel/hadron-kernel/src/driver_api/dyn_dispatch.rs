//! Dyn-compatible wrappers for async driver traits.
//!
//! The [`BlockDevice`] and [`SerialPort`] traits use `async fn` methods,
//! which makes them non-dyn-compatible. This module provides dyn-safe
//! wrapper traits that box the returned futures, enabling type-erased
//! device storage in the kernel's device registry.

extern crate alloc;

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;

use super::block::{BlockDevice, IoError};

// ---------------------------------------------------------------------------
// DynBlockDevice
// ---------------------------------------------------------------------------

/// Dyn-compatible version of [`BlockDevice`].
///
/// Wraps async trait methods in `Pin<Box<dyn Future>>` for dynamic dispatch.
/// Use [`DynBlockDeviceWrapper`] to convert any concrete [`BlockDevice`] into
/// a `Box<dyn DynBlockDevice>`.
pub trait DynBlockDevice: Send + Sync {
    /// Reads a single sector into `buf` (dyn-dispatch version).
    fn dyn_read_sector<'a>(
        &'a self,
        sector: u64,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), IoError>> + 'a>>;

    /// Writes a single sector from `buf` (dyn-dispatch version).
    fn dyn_write_sector<'a>(
        &'a self,
        sector: u64,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), IoError>> + 'a>>;

    /// Returns the size of a single sector in bytes.
    fn sector_size(&self) -> usize;

    /// Returns the total number of sectors on the device.
    fn sector_count(&self) -> u64;
}

/// Wrapper that adapts any [`BlockDevice`] into a [`DynBlockDevice`].
///
/// # Example
///
/// ```ignore
/// let disk: VirtioBlkDisk = /* ... */;
/// let dyn_disk: Box<dyn DynBlockDevice> = Box::new(DynBlockDeviceWrapper(disk));
/// ```
pub struct DynBlockDeviceWrapper<D>(pub D);

impl<D: BlockDevice> DynBlockDevice for DynBlockDeviceWrapper<D> {
    fn dyn_read_sector<'a>(
        &'a self,
        sector: u64,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), IoError>> + 'a>> {
        Box::pin(self.0.read_sector(sector, buf))
    }

    fn dyn_write_sector<'a>(
        &'a self,
        sector: u64,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), IoError>> + 'a>> {
        Box::pin(self.0.write_sector(sector, buf))
    }

    fn sector_size(&self) -> usize {
        self.0.sector_size()
    }

    fn sector_count(&self) -> u64 {
        self.0.sector_count()
    }
}

/// Implements [`BlockDevice`] for `Box<dyn DynBlockDevice>`, closing the
/// type-erasure round-trip.
///
/// This allows a `Box<dyn DynBlockDevice>` to be passed to any function
/// expecting `impl BlockDevice` (e.g., filesystem mount functions).
impl BlockDevice for Box<dyn DynBlockDevice> {
    async fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), IoError> {
        self.dyn_read_sector(sector, buf).await
    }

    async fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), IoError> {
        self.dyn_write_sector(sector, buf).await
    }

    fn sector_size(&self) -> usize {
        DynBlockDevice::sector_size(self.as_ref())
    }

    fn sector_count(&self) -> u64 {
        DynBlockDevice::sector_count(self.as_ref())
    }
}
