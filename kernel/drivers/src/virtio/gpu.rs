//! VirtIO GPU display driver.
//!
//! Implements [`Framebuffer`] for VirtIO GPU devices discovered via PCI.
//! Pixel writes go to cacheable RAM; explicit `TRANSFER_TO_HOST_2D` +
//! `RESOURCE_FLUSH` commands update the host display on [`flush_rect`].
//!
//! # References
//!
//! - Virtual I/O Device (VIRTIO) Specification 1.2, §5.7: GPU Device
//!   <https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html>

extern crate alloc;

use alloc::sync::Arc;
use core::ptr;

use hadron_kernel::addr::{PhysAddr, VirtAddr};
use hadron_kernel::driver_api::capability::DmaCapability;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::framebuffer::{Framebuffer, FramebufferInfo, PixelFormat};
use hadron_kernel::driver_api::pci::PciDeviceId;
use hadron_kernel::sync::SpinLock;

use super::VirtioDevice;
use super::pci::VirtioPciTransport;
use super::queue::{VIRTQ_DESC_F_WRITE, Virtqueue};

// ---------------------------------------------------------------------------
// PCI IDs
// ---------------------------------------------------------------------------

/// VirtIO vendor ID.
const VIRTIO_VENDOR: u16 = 0x1AF4;
/// VirtIO GPU device (modern PCI, transitional ID 0x1050).
const VIRTIO_GPU_DEVICE: u16 = 0x1050;

// ---------------------------------------------------------------------------
// VirtIO GPU protocol constants
// ---------------------------------------------------------------------------

/// Control command: create a 2D resource.
const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
/// Control command: set scanout (bind resource to display).
const VIRTIO_GPU_CMD_SET_SCANOUT: u32 = 0x0103;
/// Control command: flush resource to display.
const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32 = 0x0104;
/// Control command: transfer pixels to host.
const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
/// Control command: attach backing pages to a resource.
const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;

/// Response: success.
const VIRTIO_GPU_RESP_OK_NODATA: u32 = 0x1100;

/// Pixel format: B8G8R8X8 (32-bit BGR, X=padding).
const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32 = 2;

/// Default display resolution.
const DEFAULT_WIDTH: u32 = 1280;
/// Default display height.
const DEFAULT_HEIGHT: u32 = 720;
/// Bytes per pixel.
const BYTES_PER_PIXEL: u32 = 4;
/// Page size.
const PAGE_SIZE: u64 = 4096;

// ---------------------------------------------------------------------------
// VirtIO GPU protocol structures
// ---------------------------------------------------------------------------

/// Common header for all VirtIO GPU control commands and responses.
#[repr(C)]
#[derive(Clone, Copy)]
struct CtrlHeader {
    /// Command type (request) or response type.
    type_: u32,
    /// Flags (e.g. fence).
    flags: u32,
    /// Fence ID (for fencing; 0 = no fence).
    fence_id: u64,
    /// 3D context ID (0 = no context).
    ctx_id: u32,
    /// Ring index (VirtIO 1.2+; 0 for basic use).
    ring_idx: u8,
    /// Padding.
    _pad: [u8; 3],
}

impl CtrlHeader {
    /// Creates a new command header with the given type.
    const fn new(type_: u32) -> Self {
        Self {
            type_,
            flags: 0,
            fence_id: 0,
            ctx_id: 0,
            ring_idx: 0,
            _pad: [0; 3],
        }
    }
}

/// `RESOURCE_CREATE_2D` command payload.
#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceCreate2d {
    header: CtrlHeader,
    resource_id: u32,
    format: u32,
    width: u32,
    height: u32,
}

/// `RESOURCE_ATTACH_BACKING` command payload.
#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceAttachBacking {
    header: CtrlHeader,
    resource_id: u32,
    nr_entries: u32,
}

/// A memory entry for `RESOURCE_ATTACH_BACKING`.
#[repr(C)]
#[derive(Clone, Copy)]
struct MemEntry {
    addr: u64,
    length: u32,
    _pad: u32,
}

/// `SET_SCANOUT` command payload.
#[repr(C)]
#[derive(Clone, Copy)]
struct SetScanout {
    header: CtrlHeader,
    r: VirtioGpuRect,
    scanout_id: u32,
    resource_id: u32,
}

/// `TRANSFER_TO_HOST_2D` command payload.
#[repr(C)]
#[derive(Clone, Copy)]
struct TransferToHost2d {
    header: CtrlHeader,
    r: VirtioGpuRect,
    offset: u64,
    resource_id: u32,
    _pad: u32,
}

/// `RESOURCE_FLUSH` command payload.
#[repr(C)]
#[derive(Clone, Copy)]
struct ResourceFlush {
    header: CtrlHeader,
    r: VirtioGpuRect,
    resource_id: u32,
    _pad: u32,
}

/// A rectangle in the VirtIO GPU protocol.
#[repr(C)]
#[derive(Clone, Copy)]
struct VirtioGpuRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

// ---------------------------------------------------------------------------
// VirtioGpu driver
// ---------------------------------------------------------------------------

/// VirtIO GPU display driver implementing [`Framebuffer`].
///
/// Pixel writes target cacheable RAM. The host display is updated via
/// `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH` commands sent through the
/// control virtqueue.
pub struct VirtioGpu {
    /// The underlying VirtIO device.
    device: VirtioDevice,
    /// The control virtqueue (index 0), behind a lock for concurrent flush.
    controlq: SpinLock<Virtqueue>,
    /// Physical address of the framebuffer DMA allocation.
    fb_phys: u64,
    /// Virtual address of the framebuffer (cacheable kernel mapping via HHDM).
    fb_virt: u64,
    /// Physical address of the command/response DMA page.
    cmd_buf_phys: u64,
    /// Virtual address of the command/response DMA page.
    cmd_buf_virt: u64,
    /// Display width in pixels.
    width: u32,
    /// Display height in pixels.
    height: u32,
    /// Resource ID (always 1).
    resource_id: u32,
    /// DMA capability for memory operations.
    dma: DmaCapability,
}

// SAFETY: VirtioGpu is Send+Sync because:
// - controlq is behind SpinLock
// - fb_phys/fb_virt/cmd_buf_phys/cmd_buf_virt are immutable after init
// - DmaCapability is Copy
unsafe impl Send for VirtioGpu {}
unsafe impl Sync for VirtioGpu {}

impl VirtioGpu {
    /// Sends a command to the device and waits for the response.
    ///
    /// Copies the command to the DMA command buffer, submits a two-descriptor
    /// chain (command readable, response writable), notifies the device, and
    /// polls for completion. Returns the response type code.
    fn send_cmd<C: Copy>(&self, cmd: &C) -> u32 {
        let cmd_size = core::mem::size_of::<C>();
        let resp_size = core::mem::size_of::<CtrlHeader>();

        // Copy command to DMA buffer.
        // SAFETY: cmd_buf_virt points to a page we own; cmd_size fits in one page.
        unsafe {
            ptr::copy_nonoverlapping(
                cmd as *const C as *const u8,
                self.cmd_buf_virt as *mut u8,
                cmd_size,
            );
        }

        // Zero the response area.
        let resp_offset = cmd_size;
        // SAFETY: Response area is within the same DMA page.
        unsafe {
            ptr::write_bytes(
                (self.cmd_buf_virt as *mut u8).add(resp_offset),
                0,
                resp_size,
            );
        }

        let cmd_phys = self.cmd_buf_phys;
        let resp_phys = self.cmd_buf_phys + resp_offset as u64;

        // Build a two-descriptor chain: [cmd | NEXT] → [response | WRITE].
        let chain: [(u64, u32, u16); 2] = [
            (cmd_phys, cmd_size as u32, 0),
            (resp_phys, resp_size as u32, VIRTQ_DESC_F_WRITE),
        ];

        let mut vq = self.controlq.lock();
        vq.add_buf(&chain).expect("virtio-gpu: controlq full");
        vq.notify(self.device.transport(), 0);

        // Poll for completion.
        loop {
            if vq.poll_used().is_some() {
                break;
            }
            core::hint::spin_loop();
        }

        // Read response type.
        // SAFETY: The device has written the response header.
        let resp_type = unsafe {
            let resp_ptr = (self.cmd_buf_virt + resp_offset as u64) as *const CtrlHeader;
            (*resp_ptr).type_
        };

        resp_type
    }
}

impl Framebuffer for VirtioGpu {
    fn info(&self) -> FramebufferInfo {
        FramebufferInfo {
            width: self.width,
            height: self.height,
            pitch: self.width * BYTES_PER_PIXEL,
            bpp: 32,
            pixel_format: PixelFormat::Bgr32,
        }
    }

    fn base_address(&self) -> VirtAddr {
        VirtAddr::new(self.fb_virt)
    }

    fn physical_base(&self) -> PhysAddr {
        PhysAddr::new(self.fb_phys)
    }

    fn is_ram_backed(&self) -> bool {
        true
    }

    fn flush_rect(&self, x: u32, y: u32, w: u32, h: u32) {
        // Clamp to framebuffer bounds.
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width.saturating_sub(x));
        let h = h.min(self.height.saturating_sub(y));
        if w == 0 || h == 0 {
            return;
        }

        let rect = VirtioGpuRect {
            x,
            y,
            width: w,
            height: h,
        };

        // Transfer pixels to host.
        let transfer = TransferToHost2d {
            header: CtrlHeader::new(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
            r: rect,
            offset: 0,
            resource_id: self.resource_id,
            _pad: 0,
        };
        self.send_cmd(&transfer);

        // Flush to display.
        let flush = ResourceFlush {
            header: CtrlHeader::new(VIRTIO_GPU_CMD_RESOURCE_FLUSH),
            r: rect,
            resource_id: self.resource_id,
            _pad: 0,
        };
        self.send_cmd(&flush);
    }

    fn put_pixel(&self, x: u32, y: u32, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let pitch = self.width * BYTES_PER_PIXEL;
        let offset = (y as u64) * (pitch as u64) + (x as u64) * (BYTES_PER_PIXEL as u64);
        // SAFETY: Bounds checked above, offset is within the framebuffer.
        unsafe {
            let dst = (self.fb_virt + offset) as *mut u32;
            ptr::write(dst, color);
        }
    }

    fn write_scanline(&self, x: u32, y: u32, pixels: &[u32]) {
        let count = pixels.len() as u32;
        if count == 0 || x >= self.width || y >= self.height {
            return;
        }
        let clamped = count.min(self.width - x) as usize;
        let pitch = self.width * BYTES_PER_PIXEL;
        let offset = (y as u64) * (pitch as u64) + (x as u64) * (BYTES_PER_PIXEL as u64);
        let dst = (self.fb_virt + offset) as *mut u32;
        // SAFETY: `clamped` pixels within row bounds, non-overlapping copy.
        unsafe { ptr::copy_nonoverlapping(pixels.as_ptr(), dst, clamped) };
    }

    fn fill_rect(&self, x: u32, y: u32, width: u32, height: u32, color: u32) {
        let x_end = (x + width).min(self.width);
        let y_end = (y + height).min(self.height);
        let span = (x_end - x) as usize;
        if span == 0 || y >= y_end {
            return;
        }

        // Build one scanline of fill color, then bulk-copy to each row.
        const MAX_SPAN: usize = 256;
        let mut row_buf = [0u32; MAX_SPAN];
        let clamped = span.min(MAX_SPAN);
        for px in &mut row_buf[..clamped] {
            *px = color;
        }

        let pitch = (self.width * BYTES_PER_PIXEL) as u64;

        for row in y..y_end {
            let row_offset = (row as u64) * pitch + (x as u64) * (BYTES_PER_PIXEL as u64);
            let dst = (self.fb_virt + row_offset) as *mut u32;
            let mut written = 0;
            while written < span {
                let chunk = (span - written).min(clamped);
                // SAFETY: Bounds clamped to framebuffer dimensions above.
                unsafe { ptr::copy_nonoverlapping(row_buf.as_ptr(), dst.add(written), chunk) };
                written += chunk;
            }
        }
    }

    unsafe fn copy_within(&self, src_offset: u64, dst_offset: u64, count: usize) {
        let base = self.fb_virt as *mut u8;
        // SAFETY: Caller guarantees offsets and count are within FB bounds.
        unsafe {
            ptr::copy(
                base.add(src_offset as usize),
                base.add(dst_offset as usize),
                count,
            );
        }
    }

    unsafe fn fill_zero(&self, offset: u64, count: usize) {
        let base = self.fb_virt as *mut u8;
        // SAFETY: Caller guarantees offset and count are within FB bounds.
        unsafe {
            ptr::write_bytes(base.add(offset as usize), 0, count);
        }
    }
}

// ---------------------------------------------------------------------------
// PCI registration
// ---------------------------------------------------------------------------

/// PCI device ID table for VirtIO GPU.
#[cfg(target_os = "none")]
static ID_TABLE: [PciDeviceId; 1] = [PciDeviceId::new(VIRTIO_VENDOR, VIRTIO_GPU_DEVICE)];

/// VirtIO GPU driver registration type.
struct VirtioGpuDriver;

#[hadron_driver_macros::hadron_driver(
    name = "virtio-gpu",
    kind = pci,
    capabilities = [Mmio, Dma, PciConfig],
    pci_ids = &ID_TABLE,
)]
impl VirtioGpuDriver {
    /// PCI probe function for VirtIO GPU devices.
    fn probe(
        ctx: DriverContext,
    ) -> Result<hadron_kernel::driver_api::registration::PciDriverRegistration, DriverError> {
        use hadron_kernel::driver_api::capability::{
            CapabilityAccess, DmaCapability, MmioCapability, PciConfigCapability,
        };
        use hadron_kernel::driver_api::device_path::DevicePath;
        use hadron_kernel::driver_api::registration::{DeviceSet, PciDriverRegistration};

        let info = ctx.device();
        let pci_config = ctx.capability::<PciConfigCapability>();
        let mmio_cap = ctx.capability::<MmioCapability>();
        let dma = ctx.capability::<DmaCapability>();

        hadron_kernel::kinfo!(
            "virtio-gpu: probing {:04x}:{:04x} at {}",
            info.vendor_id,
            info.device_id,
            info.address
        );

        // Enable bus mastering for DMA.
        pci_config.enable_bus_mastering();

        // Initialize VirtIO PCI transport.
        let transport = VirtioPciTransport::new(info, mmio_cap)?;

        // Initialize VirtIO device (steps 1-6).
        // No device-specific feature bits needed for basic 2D rendering.
        let device = VirtioDevice::init(transport, 0)?;

        // Setup control queue (queue 0).
        let controlq = device.setup_queue(0, dma)?;

        // Allocate framebuffer: width * height * 4 bytes, page-aligned.
        let fb_bytes = (DEFAULT_WIDTH as u64) * (DEFAULT_HEIGHT as u64) * (BYTES_PER_PIXEL as u64);
        let fb_pages = ((fb_bytes + PAGE_SIZE - 1) / PAGE_SIZE) as usize;
        let fb_phys = dma.alloc_frames(fb_pages)?;
        let fb_virt = dma.phys_to_virt(fb_phys);

        // Zero the framebuffer.
        // SAFETY: Freshly allocated DMA pages.
        unsafe {
            ptr::write_bytes(fb_virt as *mut u8, 0, fb_pages * PAGE_SIZE as usize);
        }

        // Allocate one page for command/response buffers.
        let cmd_buf_phys = dma.alloc_frames(1)?;
        let cmd_buf_virt = dma.phys_to_virt(cmd_buf_phys);

        // Complete VirtIO init.
        device.set_driver_ok();

        let gpu = VirtioGpu {
            device,
            controlq: SpinLock::named("VirtioGpu.controlq", controlq),
            fb_phys,
            fb_virt,
            cmd_buf_phys,
            cmd_buf_virt,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            resource_id: 1,
            dma: *dma,
        };

        // Send GPU init commands.

        // 1. Create 2D resource.
        let create = ResourceCreate2d {
            header: CtrlHeader::new(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
            resource_id: 1,
            format: VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        };
        let resp = gpu.send_cmd(&create);
        if resp != VIRTIO_GPU_RESP_OK_NODATA {
            hadron_kernel::kwarn!("virtio-gpu: RESOURCE_CREATE_2D failed (resp={:#x})", resp);
            return Err(DriverError::InitFailed);
        }

        // 2. Attach backing pages.
        // The attach_backing command is immediately followed by the mem entries.
        // We manually build this in the command buffer.
        {
            let cmd_ptr = gpu.cmd_buf_virt as *mut u8;
            let attach = ResourceAttachBacking {
                header: CtrlHeader::new(VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING),
                resource_id: 1,
                nr_entries: 1,
            };
            let entry = MemEntry {
                addr: fb_phys,
                length: fb_bytes as u32,
                _pad: 0,
            };

            let attach_size = core::mem::size_of::<ResourceAttachBacking>();
            let entry_size = core::mem::size_of::<MemEntry>();
            let total_cmd_size = attach_size + entry_size;
            let resp_size = core::mem::size_of::<CtrlHeader>();

            // Write attach header + mem entry contiguously.
            // SAFETY: cmd_buf is a full page, total_cmd_size + resp_size fits.
            unsafe {
                ptr::copy_nonoverlapping(
                    &attach as *const ResourceAttachBacking as *const u8,
                    cmd_ptr,
                    attach_size,
                );
                ptr::copy_nonoverlapping(
                    &entry as *const MemEntry as *const u8,
                    cmd_ptr.add(attach_size),
                    entry_size,
                );
                // Zero response area.
                ptr::write_bytes(cmd_ptr.add(total_cmd_size), 0, resp_size);
            }

            let cmd_phys = gpu.cmd_buf_phys;
            let resp_phys = gpu.cmd_buf_phys + total_cmd_size as u64;

            let chain: [(u64, u32, u16); 2] = [
                (cmd_phys, total_cmd_size as u32, 0),
                (resp_phys, resp_size as u32, VIRTQ_DESC_F_WRITE),
            ];

            let mut vq = gpu.controlq.lock();
            vq.add_buf(&chain).expect("virtio-gpu: controlq full");
            vq.notify(gpu.device.transport(), 0);

            loop {
                if vq.poll_used().is_some() {
                    break;
                }
                core::hint::spin_loop();
            }

            // SAFETY: Device has written the response.
            let resp_type = unsafe {
                let resp_ptr = (gpu.cmd_buf_virt + total_cmd_size as u64) as *const CtrlHeader;
                (*resp_ptr).type_
            };
            if resp_type != VIRTIO_GPU_RESP_OK_NODATA {
                hadron_kernel::kwarn!(
                    "virtio-gpu: RESOURCE_ATTACH_BACKING failed (resp={:#x})",
                    resp_type
                );
                return Err(DriverError::InitFailed);
            }
        }

        // 3. Set scanout.
        let scanout = SetScanout {
            header: CtrlHeader::new(VIRTIO_GPU_CMD_SET_SCANOUT),
            r: VirtioGpuRect {
                x: 0,
                y: 0,
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
            },
            scanout_id: 0,
            resource_id: 1,
        };
        let resp = gpu.send_cmd(&scanout);
        if resp != VIRTIO_GPU_RESP_OK_NODATA {
            hadron_kernel::kwarn!("virtio-gpu: SET_SCANOUT failed (resp={:#x})", resp);
            return Err(DriverError::InitFailed);
        }

        hadron_kernel::kinfo!(
            "virtio-gpu: initialized {}x{} display, fb at {:#x} (phys {:#x})",
            DEFAULT_WIDTH,
            DEFAULT_HEIGHT,
            fb_virt,
            fb_phys
        );

        let gpu_arc = Arc::new(gpu);

        let mut devices = DeviceSet::new();
        let path = DevicePath::pci(
            info.address.bus,
            info.address.device,
            info.address.function,
            "virtio-gpu",
            0,
        );
        devices.add_framebuffer(path, gpu_arc);

        hadron_kernel::kinfo!("virtio-gpu: driver initialized successfully");
        Ok(PciDriverRegistration {
            devices,
            lifecycle: None,
        })
    }
}
