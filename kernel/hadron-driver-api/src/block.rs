//! Block device interface trait and error types.
//!
//! Defines the [`BlockDevice`] trait for async sector-level I/O, used by
//! storage drivers such as AHCI SATA.

use core::fmt;

/// Errors that can occur during block I/O operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoError {
    /// The requested sector is beyond the device's capacity.
    OutOfRange,
    /// The device reported a hardware error.
    DeviceError,
    /// The provided buffer is not the correct size for the operation.
    InvalidBuffer,
    /// The operation timed out waiting for the device.
    Timeout,
    /// A DMA buffer allocation or setup error occurred.
    DmaError,
    /// The device is not ready to accept commands.
    NotReady,
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange => f.write_str("sector out of range"),
            Self::DeviceError => f.write_str("device error"),
            Self::InvalidBuffer => f.write_str("invalid buffer size"),
            Self::Timeout => f.write_str("operation timed out"),
            Self::DmaError => f.write_str("DMA error"),
            Self::NotReady => f.write_str("device not ready"),
        }
    }
}

/// Async block device interface for sector-level I/O.
///
/// Drivers implementing this trait provide read/write access to block storage
/// devices. All I/O operations are async to allow cooperative scheduling while
/// waiting for hardware completion.
#[allow(async_fn_in_trait)]
pub trait BlockDevice: Send + Sync {
    /// Reads a single sector into `buf`.
    ///
    /// `buf` must be exactly [`sector_size()`](Self::sector_size) bytes long.
    async fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), IoError>;

    /// Writes a single sector from `buf`.
    ///
    /// `buf` must be exactly [`sector_size()`](Self::sector_size) bytes long.
    async fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), IoError>;

    /// Returns the size of a single sector in bytes (typically 512).
    fn sector_size(&self) -> usize;

    /// Returns the total number of sectors on the device.
    fn sector_count(&self) -> u64;

    /// Reads `count` consecutive sectors starting at `start_sector` into `buf`.
    ///
    /// Default implementation reads one sector at a time.
    async fn read_sectors(
        &self,
        start_sector: u64,
        count: u64,
        buf: &mut [u8],
    ) -> Result<(), IoError> {
        let ss = self.sector_size();
        if buf.len() < ss * count as usize {
            return Err(IoError::InvalidBuffer);
        }
        for i in 0..count {
            let offset = i as usize * ss;
            self.read_sector(start_sector + i, &mut buf[offset..offset + ss])
                .await?;
        }
        Ok(())
    }
}
