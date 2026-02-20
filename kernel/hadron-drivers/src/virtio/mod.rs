//! VirtIO device support.
//!
//! Provides VirtIO PCI modern transport, split virtqueue management, and
//! the standard device initialization sequence per the VirtIO 1.0 spec.

extern crate alloc;

pub mod block;
pub mod pci;
pub mod queue;

use hadron_kernel::driver_api::capability::DmaCapability;
use hadron_kernel::driver_api::error::DriverError;

use pci::VirtioPciTransport;
use queue::Virtqueue;

// -- VirtIO device status bits ------------------------------------------------

/// Device status: ACKNOWLEDGE — guest OS has found the device.
const STATUS_ACKNOWLEDGE: u8 = 1;
/// Device status: DRIVER — guest OS knows how to drive the device.
const STATUS_DRIVER: u8 = 2;
/// Device status: DRIVER_OK — driver is ready.
const STATUS_DRIVER_OK: u8 = 4;
/// Device status: FEATURES_OK — feature negotiation complete.
const STATUS_FEATURES_OK: u8 = 8;
/// Device status: FAILED — something went wrong.
const STATUS_FAILED: u8 = 128;

// -- VirtIO feature bits ------------------------------------------------------

/// `VIRTIO_F_VERSION_1` (bit 32) — device complies with VirtIO 1.0 spec.
const VIRTIO_F_VERSION_1: u32 = 1 << 0; // bit 32 is bit 0 of the high dword

/// MSI-X "no vector" sentinel value.
pub const VIRTIO_MSI_NO_VECTOR: u16 = 0xFFFF;

/// A VirtIO device initialized via PCI modern transport.
///
/// Wraps the transport and negotiated state. Used by device-specific drivers
/// (e.g., `virtio-blk`) to interact with the device.
pub struct VirtioDevice {
    /// PCI modern transport.
    transport: VirtioPciTransport,
}

impl VirtioDevice {
    /// Performs the standard VirtIO 1.0 initialization sequence.
    ///
    /// Steps 1-6 of the spec (reset → acknowledge → driver → feature
    /// negotiation → features_ok). The caller must complete device-specific
    /// setup (step 7) and then call [`set_driver_ok`](Self::set_driver_ok).
    ///
    /// `device_features_mask` specifies which device-specific features (bits
    /// 0-31 of the low dword) the driver wants to negotiate.
    pub fn init(
        transport: VirtioPciTransport,
        device_features_mask: u32,
    ) -> Result<Self, DriverError> {
        // Step 1: Reset the device.
        transport.set_device_status(0);

        // Step 2: Set ACKNOWLEDGE.
        transport.set_device_status(STATUS_ACKNOWLEDGE);

        // Step 3: Set DRIVER.
        transport.set_device_status(STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        // Step 4: Read device features and negotiate.
        // Read low 32 bits (device-specific features).
        transport.set_device_feature_select(0);
        let dev_features_lo = transport.device_feature();

        // Read high 32 bits (must include VIRTIO_F_VERSION_1).
        transport.set_device_feature_select(1);
        let dev_features_hi = transport.device_feature();

        if dev_features_hi & VIRTIO_F_VERSION_1 == 0 {
            hadron_kernel::kwarn!("virtio: device does not support VIRTIO_F_VERSION_1");
            transport.set_device_status(STATUS_FAILED);
            return Err(DriverError::Unsupported);
        }

        // Accept requested device features.
        let driver_features_lo = dev_features_lo & device_features_mask;
        transport.set_driver_feature_select(0);
        transport.set_driver_feature(driver_features_lo);

        // Accept VERSION_1 in high dword.
        transport.set_driver_feature_select(1);
        transport.set_driver_feature(VIRTIO_F_VERSION_1);

        // Step 5: Set FEATURES_OK.
        let status = STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK;
        transport.set_device_status(status);

        // Step 6: Re-read status to verify FEATURES_OK is still set.
        let readback = transport.device_status();
        if readback & STATUS_FEATURES_OK == 0 {
            hadron_kernel::kwarn!("virtio: device rejected features");
            transport.set_device_status(STATUS_FAILED);
            return Err(DriverError::InitFailed);
        }

        Ok(Self { transport })
    }

    /// Sets the DRIVER_OK bit, completing initialization.
    ///
    /// Call this after device-specific setup (queue init, MSI-X config).
    pub fn set_driver_ok(&self) {
        let status = self.transport.device_status();
        self.transport.set_device_status(status | STATUS_DRIVER_OK);
    }

    /// Returns a reference to the underlying PCI transport.
    #[must_use]
    pub fn transport(&self) -> &VirtioPciTransport {
        &self.transport
    }

    /// Sets up a virtqueue at the given index.
    ///
    /// Selects the queue, reads its max size, allocates DMA memory, and
    /// programs the descriptor/avail/used addresses into the device.
    pub fn setup_queue(
        &self,
        queue_index: u16,
        dma: &DmaCapability,
    ) -> Result<Virtqueue, DriverError> {
        let t = &self.transport;

        // Select the queue.
        t.set_queue_select(queue_index);

        // Read the maximum queue size supported by the device.
        let max_size = t.queue_size();
        if max_size == 0 {
            return Err(DriverError::InitFailed);
        }

        // Use the device's maximum (or cap at 256 for sanity).
        let queue_size = max_size.min(256);
        t.set_queue_size(queue_size);

        // Allocate the virtqueue.
        let vq = Virtqueue::new(queue_size, dma)?;

        // Program addresses.
        t.set_queue_desc(vq.desc_phys());
        t.set_queue_avail(vq.avail_phys());
        t.set_queue_used(vq.used_phys());

        // Enable the queue.
        t.set_queue_enable(1);

        Ok(vq)
    }
}
