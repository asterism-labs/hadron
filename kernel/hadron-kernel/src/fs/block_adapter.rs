//! Adapter bridging async [`BlockDevice`] to synchronous `hadris_io` traits.
//!
//! [`BlockDeviceAdapter`] wraps any [`BlockDevice`] and implements
//! [`hadris_io::Read`], [`hadris_io::Seek`], and [`hadris_io::Write`] by using
//! [`block_on`](crate::sched::block_on::block_on) to synchronously poll the
//! async sector I/O methods. This allows hadris-fat and hadris-iso to read from
//! kernel block devices.

extern crate alloc;

use alloc::vec;

use hadris_io::{Error, ErrorKind, Read, Result, Seek, SeekFrom, Write};
use crate::driver_api::block::BlockDevice;

use crate::sched::block_on::block_on;

/// Type alias for a block device adapter wrapping a type-erased block device.
///
/// Used by filesystem registration entries ([`BlockFsEntry`](crate::driver_api::registration::BlockFsEntry))
/// to pass block devices to filesystem mount functions without generic parameters.
pub type BoxedBlockAdapter =
    BlockDeviceAdapter<alloc::boxed::Box<dyn crate::driver_api::dyn_dispatch::DynBlockDevice>>;

/// Adapts an async [`BlockDevice`] into synchronous `hadris_io::Read + Seek + Write`.
///
/// Maintains a byte-level cursor position and a sector-sized scratch buffer for
/// translating byte-oriented I/O into sector-aligned block device operations.
/// Each `read`/`write` call processes at most one sector's worth of data; the
/// `read_exact`/`write_all` default methods in `hadris_io` loop as needed.
pub struct BlockDeviceAdapter<D: BlockDevice> {
    /// The underlying block device.
    device: D,
    /// Current byte position within the device.
    position: u64,
    /// Scratch buffer for single-sector reads and read-modify-write cycles.
    sector_buf: alloc::vec::Vec<u8>,
    /// Total device size in bytes (`sector_count * sector_size`).
    total_size: u64,
}

impl<D: BlockDevice> BlockDeviceAdapter<D> {
    /// Creates a new adapter over the given block device.
    ///
    /// Allocates a sector-sized scratch buffer on the heap.
    #[must_use]
    pub fn new(device: D) -> Self {
        let sector_size = device.sector_size();
        let total_size = device.sector_count() * sector_size as u64;
        Self {
            device,
            position: 0,
            sector_buf: vec![0u8; sector_size],
            total_size,
        }
    }
}

impl<D: BlockDevice> Read for BlockDeviceAdapter<D> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() || self.position >= self.total_size {
            return Ok(0);
        }

        let sector_size = self.device.sector_size() as u64;
        let sector = self.position / sector_size;
        let offset_in_sector = (self.position % sector_size) as usize;

        block_on(self.device.read_sector(sector, &mut self.sector_buf))
            .map_err(|_| Error::from_kind(ErrorKind::Other))?;

        let available_in_sector = self.device.sector_size() - offset_in_sector;
        let remaining_on_device = (self.total_size - self.position) as usize;
        let to_copy = buf.len().min(available_in_sector).min(remaining_on_device);

        buf[..to_copy].copy_from_slice(&self.sector_buf[offset_in_sector..offset_in_sector + to_copy]);
        self.position += to_copy as u64;

        Ok(to_copy)
    }
}

impl<D: BlockDevice> Seek for BlockDeviceAdapter<D> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.total_size as i64 + offset,
            SeekFrom::Current(offset) => self.position as i64 + offset,
        };

        if new_pos < 0 {
            return Err(Error::new(ErrorKind::InvalidInput, "seek to negative position"));
        }

        self.position = new_pos as u64;
        Ok(self.position)
    }
}

impl<D: BlockDevice> Write for BlockDeviceAdapter<D> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() || self.position >= self.total_size {
            return Ok(0);
        }

        let sector_size = self.device.sector_size() as u64;
        let sector = self.position / sector_size;
        let offset_in_sector = (self.position % sector_size) as usize;

        // Read-modify-write: read existing sector, overlay new data, write back.
        block_on(self.device.read_sector(sector, &mut self.sector_buf))
            .map_err(|_| Error::from_kind(ErrorKind::Other))?;

        let available_in_sector = self.device.sector_size() - offset_in_sector;
        let remaining_on_device = (self.total_size - self.position) as usize;
        let to_write = buf.len().min(available_in_sector).min(remaining_on_device);

        self.sector_buf[offset_in_sector..offset_in_sector + to_write]
            .copy_from_slice(&buf[..to_write]);

        block_on(self.device.write_sector(sector, &self.sector_buf))
            .map_err(|_| Error::from_kind(ErrorKind::Other))?;

        self.position += to_write as u64;
        Ok(to_write)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
