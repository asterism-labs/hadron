//! In-memory block device for testing.
//!
//! [`RamDisk`] implements [`BlockDevice`] using a heap-allocated `Vec<u8>` as
//! backing storage. No IRQ, no DMA — useful for unit tests and filesystem
//! testing without hardware.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use hadron_kernel::driver_api::block::{BlockDevice, IoError};

/// An in-memory block device backed by a `Vec<u8>`.
pub struct RamDisk {
    /// Backing storage.
    data: Vec<u8>,
    /// Bytes per sector.
    sector_size: usize,
    /// Total number of sectors.
    sector_count: u64,
}

impl RamDisk {
    /// Creates a new ramdisk with the given sector count and size.
    ///
    /// Allocates `sector_count * sector_size` bytes of zeroed memory.
    #[must_use]
    pub fn new(sector_count: u64, sector_size: usize) -> Self {
        let total_bytes = sector_count as usize * sector_size;
        Self {
            data: vec![0u8; total_bytes],
            sector_size,
            sector_count,
        }
    }

    /// Returns a slice of the raw backing data.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Returns a mutable slice of the raw backing data.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

impl BlockDevice for RamDisk {
    async fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), IoError> {
        if sector >= self.sector_count {
            return Err(IoError::OutOfRange);
        }
        if buf.len() < self.sector_size {
            return Err(IoError::InvalidBuffer);
        }

        let offset = sector as usize * self.sector_size;
        buf[..self.sector_size].copy_from_slice(&self.data[offset..offset + self.sector_size]);
        Ok(())
    }

    async fn write_sector(&self, _sector: u64, _buf: &[u8]) -> Result<(), IoError> {
        // RamDisk is behind a shared reference in BlockDevice, so write
        // requires interior mutability. For simplicity, write is not supported
        // through the trait — use `as_bytes_mut()` for direct mutation.
        Err(IoError::NotReady)
    }

    fn sector_size(&self) -> usize {
        self.sector_size
    }

    fn sector_count(&self) -> u64 {
        self.sector_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: core::future::Future>(f: F) -> F::Output {
        use core::pin::pin;
        use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

        fn noop_clone(_: *const ()) -> RawWaker {
            RawWaker::new(core::ptr::null(), &VTABLE)
        }
        fn noop(_: *const ()) {}
        static VTABLE: RawWakerVTable = RawWakerVTable::new(noop_clone, noop, noop, noop);

        let waker = unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = pin!(f);

        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(val) => val,
            Poll::Pending => panic!("RamDisk future should complete immediately"),
        }
    }

    #[test]
    fn read_write_roundtrip() {
        let mut disk = RamDisk::new(8, 512);

        // Write test data directly.
        let sector_data = [0xABu8; 512];
        disk.as_bytes_mut()[0..512].copy_from_slice(&sector_data);

        // Read it back through the BlockDevice trait.
        let mut buf = [0u8; 512];
        block_on(disk.read_sector(0, &mut buf)).expect("read should succeed");
        assert_eq!(buf, sector_data);
    }

    #[test]
    fn read_out_of_range() {
        let disk = RamDisk::new(4, 512);
        let mut buf = [0u8; 512];
        let err = block_on(disk.read_sector(4, &mut buf)).unwrap_err();
        assert_eq!(err, IoError::OutOfRange);
    }

    #[test]
    fn read_invalid_buffer() {
        let disk = RamDisk::new(4, 512);
        let mut buf = [0u8; 256]; // too small
        let err = block_on(disk.read_sector(0, &mut buf)).unwrap_err();
        assert_eq!(err, IoError::InvalidBuffer);
    }

    #[test]
    fn sector_count_and_size() {
        let disk = RamDisk::new(16, 4096);
        assert_eq!(disk.sector_count(), 16);
        assert_eq!(disk.sector_size(), 4096);
    }

    #[test]
    fn multiple_sectors() {
        let mut disk = RamDisk::new(4, 512);

        // Write different patterns to each sector.
        for i in 0..4u8 {
            let offset = i as usize * 512;
            disk.as_bytes_mut()[offset..offset + 512].fill(i + 1);
        }

        // Read each sector and verify.
        for i in 0..4u64 {
            let mut buf = [0u8; 512];
            block_on(disk.read_sector(i, &mut buf)).expect("read should succeed");
            assert!(buf.iter().all(|&b| b == (i as u8 + 1)));
        }
    }
}
