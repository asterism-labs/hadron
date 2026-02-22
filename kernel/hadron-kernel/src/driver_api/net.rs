//! Network device interface trait and error types.
//!
//! Defines the [`NetworkDevice`] trait for async Ethernet frame I/O, used by
//! network drivers such as VirtIO-net.

use core::fmt;

/// A 6-byte MAC (Ethernet hardware) address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddress(pub [u8; 6]);

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", a, b, c, d, e, g)
    }
}

/// Errors that can occur during network I/O operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    /// The device reported a hardware error.
    DeviceError,
    /// The provided receive buffer is too small for the incoming frame.
    BufferTooSmall,
    /// The packet exceeds the device's MTU.
    PacketTooLarge,
    /// A DMA buffer allocation or setup error occurred.
    DmaError,
    /// The device is not ready to accept commands.
    NotReady,
    /// No packet is available (non-blocking context).
    WouldBlock,
    /// The transmit queue is full.
    TxQueueFull,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceError => f.write_str("device error"),
            Self::BufferTooSmall => f.write_str("receive buffer too small"),
            Self::PacketTooLarge => f.write_str("packet exceeds MTU"),
            Self::DmaError => f.write_str("DMA error"),
            Self::NotReady => f.write_str("device not ready"),
            Self::WouldBlock => f.write_str("would block"),
            Self::TxQueueFull => f.write_str("transmit queue full"),
        }
    }
}

/// Async network device interface for Ethernet frame I/O.
///
/// Drivers implementing this trait provide send/receive access to network
/// devices. All I/O operations are async to allow cooperative scheduling while
/// waiting for hardware completion.
#[expect(async_fn_in_trait, reason = "internal trait, no dyn dispatch needed")]
pub trait NetworkDevice: Send + Sync {
    /// Receives a single Ethernet frame into `buf`.
    ///
    /// Returns the number of bytes written to `buf`. Blocks (async) until a
    /// frame is available.
    async fn recv(&self, buf: &mut [u8]) -> Result<usize, NetError>;

    /// Sends a single Ethernet frame from `buf`.
    async fn send(&self, buf: &[u8]) -> Result<(), NetError>;

    /// Returns the device's MAC address.
    fn mac_address(&self) -> MacAddress;

    /// Returns the maximum transmission unit (Ethernet header + payload).
    ///
    /// Default is 1514 bytes (14-byte Ethernet header + 1500-byte payload).
    fn mtu(&self) -> usize {
        1514
    }
}
