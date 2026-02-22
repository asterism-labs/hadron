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
use super::net::{MacAddress, NetError, NetworkDevice};

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

// ---------------------------------------------------------------------------
// DynNetDevice
// ---------------------------------------------------------------------------

/// Dyn-compatible version of [`NetworkDevice`].
///
/// Wraps async trait methods in `Pin<Box<dyn Future>>` for dynamic dispatch.
/// Use [`DynNetDeviceWrapper`] to convert any concrete [`NetworkDevice`] into
/// a `Box<dyn DynNetDevice>`.
pub trait DynNetDevice: Send + Sync {
    /// Receives a single Ethernet frame into `buf` (dyn-dispatch version).
    fn dyn_recv<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, NetError>> + 'a>>;

    /// Sends a single Ethernet frame from `buf` (dyn-dispatch version).
    fn dyn_send<'a>(
        &'a self,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), NetError>> + 'a>>;

    /// Returns the device's MAC address.
    fn mac_address(&self) -> MacAddress;

    /// Returns the maximum transmission unit.
    fn mtu(&self) -> usize;
}

/// Wrapper that adapts any [`NetworkDevice`] into a [`DynNetDevice`].
pub struct DynNetDeviceWrapper<D>(pub D);

impl<D: NetworkDevice> DynNetDevice for DynNetDeviceWrapper<D> {
    fn dyn_recv<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, NetError>> + 'a>> {
        Box::pin(self.0.recv(buf))
    }

    fn dyn_send<'a>(
        &'a self,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), NetError>> + 'a>> {
        Box::pin(self.0.send(buf))
    }

    fn mac_address(&self) -> MacAddress {
        self.0.mac_address()
    }

    fn mtu(&self) -> usize {
        self.0.mtu()
    }
}

/// Implements [`NetworkDevice`] for `Box<dyn DynNetDevice>`, closing the
/// type-erasure round-trip.
impl NetworkDevice for Box<dyn DynNetDevice> {
    async fn recv(&self, buf: &mut [u8]) -> Result<usize, NetError> {
        self.dyn_recv(buf).await
    }

    async fn send(&self, buf: &[u8]) -> Result<(), NetError> {
        self.dyn_send(buf).await
    }

    fn mac_address(&self) -> MacAddress {
        DynNetDevice::mac_address(self.as_ref())
    }

    fn mtu(&self) -> usize {
        DynNetDevice::mtu(self.as_ref())
    }
}
