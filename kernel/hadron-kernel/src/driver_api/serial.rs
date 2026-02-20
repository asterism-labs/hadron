//! Serial port interface trait.

use super::error::DriverError;

/// Interface trait for serial port devices.
///
/// Provides byte-level read/write access to a serial port. Methods take `&self`
/// because hardware I/O is inherently shared-state; callers use external
/// synchronization (e.g., `SpinLock`) when needed.
///
/// `read_byte` and `write_byte` are async to support interrupt-driven I/O.
/// `data_available` and `can_write` stay sync as they are non-blocking
/// register checks.
#[expect(async_fn_in_trait, reason = "internal trait, no dyn dispatch needed")]
pub trait SerialPort {
    /// Writes a single byte to the serial port.
    ///
    /// Async to permit implementations with flow control or back-pressure.
    /// Simple implementations may complete synchronously.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError`] if the write fails.
    async fn write_byte(&self, byte: u8) -> Result<(), DriverError>;

    /// Reads a single byte from the serial port, waiting for data if necessary.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError`] if the read fails.
    async fn read_byte(&self) -> Result<u8, DriverError>;

    /// Returns `true` if there is data available to read.
    fn data_available(&self) -> bool;

    /// Returns `true` if the transmit buffer can accept a byte.
    fn can_write(&self) -> bool;

    /// Writes a slice of bytes to the serial port.
    ///
    /// Default implementation calls [`write_byte`](Self::write_byte) in a loop.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError`] on the first byte that fails to write.
    async fn write_bytes(&self, bytes: &[u8]) -> Result<(), DriverError> {
        for &byte in bytes {
            self.write_byte(byte).await?;
        }
        Ok(())
    }
}
