//! Bochs VGA (BGA) PCI display driver.
//!
//! Drives the Bochs/QEMU VGA adapter (vendor 0x1234, device 0x1111) using the
//! VBE DISPI interface for mode setting and a PCI BAR0 linear framebuffer.
//! Implements [`Framebuffer`] for pixel-level access.

extern crate alloc;

use core::ptr;

use hadron_kernel::addr::VirtAddr;
use hadron_kernel::arch::x86_64::Port;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::framebuffer::{Framebuffer, FramebufferInfo, PixelFormat};
use hadron_kernel::driver_api::pci::{PciBar, PciDeviceId, PciDeviceInfo};
use hadron_kernel::driver_api::resource::MmioRegion;
use hadron_kernel::driver_api::services::KernelServices;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Bochs VGA PCI vendor ID.
const BGA_VENDOR_ID: u16 = 0x1234;
/// Bochs VGA PCI device ID.
const BGA_DEVICE_ID: u16 = 0x1111;

/// VBE DISPI index I/O port.
const VBE_DISPI_INDEX_PORT: u16 = 0x01CE;
/// VBE DISPI data I/O port.
const VBE_DISPI_DATA_PORT: u16 = 0x01CF;

/// DISPI register indices.
const DISPI_INDEX_ID: u16 = 0x00;
const DISPI_INDEX_XRES: u16 = 0x01;
const DISPI_INDEX_YRES: u16 = 0x02;
const DISPI_INDEX_BPP: u16 = 0x03;
const DISPI_INDEX_ENABLE: u16 = 0x04;
#[allow(dead_code, reason = "reserved for Phase 10 mode switching")]
const DISPI_INDEX_BANK: u16 = 0x05;
#[allow(dead_code, reason = "reserved for Phase 10 mode switching")]
const DISPI_INDEX_VIRT_WIDTH: u16 = 0x06;
#[allow(dead_code, reason = "reserved for Phase 10 mode switching")]
const DISPI_INDEX_VIRT_HEIGHT: u16 = 0x07;
#[allow(dead_code, reason = "reserved for Phase 10 mode switching")]
const DISPI_INDEX_X_OFFSET: u16 = 0x08;
#[allow(dead_code, reason = "reserved for Phase 10 mode switching")]
const DISPI_INDEX_Y_OFFSET: u16 = 0x09;

/// DISPI enable register flags.
const DISPI_ENABLED: u16 = 0x01;
const DISPI_LFB_ENABLED: u16 = 0x40;
#[allow(dead_code, reason = "reserved for Phase 10 mode switching")]
const DISPI_NOCLEARMEM: u16 = 0x80;

/// Minimum expected BGA version (0xB0C0).
const BGA_VERSION_MIN: u16 = 0xB0C0;

/// Default display mode.
const DEFAULT_WIDTH: u16 = 1024;
const DEFAULT_HEIGHT: u16 = 768;
const DEFAULT_BPP: u16 = 32;
/// Bytes per pixel for 32bpp.
const BYTES_PER_PIXEL: u32 = 4;

// ---------------------------------------------------------------------------
// VBE DISPI port wrapper
// ---------------------------------------------------------------------------

/// Wrapper for the VBE DISPI index/data register pair.
struct DispiPorts {
    index: Port<u16>,
    data: Port<u16>,
}

impl DispiPorts {
    /// Creates a new DISPI port pair.
    const fn new() -> Self {
        Self {
            index: Port::new(VBE_DISPI_INDEX_PORT),
            data: Port::new(VBE_DISPI_DATA_PORT),
        }
    }

    /// Reads a DISPI register.
    ///
    /// # Safety
    ///
    /// Port I/O side effects; caller must ensure no concurrent DISPI access.
    unsafe fn read(&self, index: u16) -> u16 {
        unsafe {
            self.index.write(index);
            self.data.read()
        }
    }

    /// Writes a DISPI register.
    ///
    /// # Safety
    ///
    /// Port I/O side effects; caller must ensure no concurrent DISPI access.
    unsafe fn write(&self, index: u16, value: u16) {
        unsafe {
            self.index.write(index);
            self.data.write(value);
        }
    }
}

// ---------------------------------------------------------------------------
// BochsVga driver
// ---------------------------------------------------------------------------

/// Bochs VGA display driver state.
pub struct BochsVga {
    /// MMIO region for the linear framebuffer (BAR0).
    fb_region: MmioRegion,
    /// VBE DISPI I/O ports.
    #[allow(dead_code, reason = "reserved for Phase 10 mode switching")]
    dispi: DispiPorts,
    /// Current framebuffer metadata.
    info: FramebufferInfo,
}

// SAFETY: BochsVga is Send+Sync because:
// - fb_region is a plain data descriptor (no interior mutability)
// - dispi ports use stateless I/O port access (same as Uart16550)
// - info is plain data
// All mutable access goes through the global SpinLock.
unsafe impl Send for BochsVga {}
unsafe impl Sync for BochsVga {}

impl BochsVga {
    /// Initializes the Bochs VGA driver from PCI device info.
    ///
    /// Validates the BGA version, maps BAR0, and sets the display mode.
    fn init(
        info: &PciDeviceInfo,
        services: &dyn KernelServices,
        width: u16,
        height: u16,
        bpp: u16,
    ) -> Result<Self, DriverError> {
        let dispi = DispiPorts::new();

        // SAFETY: Reading the BGA version register via DISPI ports.
        let version = unsafe { dispi.read(DISPI_INDEX_ID) };
        if version < BGA_VERSION_MIN {
            hadron_kernel::kwarn!(
                "bochs-vga: unsupported BGA version {:#06x} (need >= {:#06x})",
                version,
                BGA_VERSION_MIN
            );
            return Err(DriverError::Unsupported);
        }
        hadron_kernel::kinfo!("bochs-vga: BGA version {:#06x}", version);

        // Extract BAR0 (framebuffer memory).
        let (bar0_phys, bar0_size) = match info.bars[0] {
            PciBar::Memory { base, size, .. } => (base, size),
            _ => return Err(DriverError::InitFailed),
        };

        // Map the framebuffer MMIO region.
        let fb_region = services.map_mmio(bar0_phys, bar0_size)?;

        // Set display mode via DISPI registers.
        // SAFETY: Writing DISPI registers to configure the display mode.
        unsafe {
            // Disable display during mode change.
            dispi.write(DISPI_INDEX_ENABLE, 0);
            // Set resolution and color depth.
            dispi.write(DISPI_INDEX_XRES, width);
            dispi.write(DISPI_INDEX_YRES, height);
            dispi.write(DISPI_INDEX_BPP, bpp);
            // Enable display with linear framebuffer mode.
            dispi.write(DISPI_INDEX_ENABLE, DISPI_ENABLED | DISPI_LFB_ENABLED);
        }

        let pitch = u32::from(width) * BYTES_PER_PIXEL;

        // Explicitly zero the framebuffer after mode switch. Hardware clear
        // (DISPI without NOCLEARMEM) is unreliable with LFB mode in some
        // emulators, leaving stale pixel data visible.
        let fb_byte_count = pitch as usize * usize::from(height);
        // SAFETY: The framebuffer region was just mapped and is writable.
        // `fb_byte_count` is bounded by the BAR0 size (checked by map_mmio).
        unsafe {
            ptr::write_bytes(fb_region.virt_base().as_u64() as *mut u8, 0, fb_byte_count);
        }

        let fb_info = FramebufferInfo {
            width: u32::from(width),
            height: u32::from(height),
            pitch,
            bpp: bpp as u8,
            pixel_format: PixelFormat::Bgr32,
        };

        hadron_kernel::kinfo!(
            "bochs-vga: mode set {}x{}x{}, pitch={}, fb at {:#x}",
            width,
            height,
            bpp,
            pitch,
            fb_region.virt_base().as_u64()
        );

        Ok(Self {
            fb_region,
            dispi,
            info: fb_info,
        })
    }
}

impl Framebuffer for BochsVga {
    fn info(&self) -> FramebufferInfo {
        self.info
    }

    fn base_address(&self) -> VirtAddr {
        self.fb_region.virt_base()
    }

    fn put_pixel(&self, x: u32, y: u32, color: u32) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }
        let offset = (y as u64) * (self.info.pitch as u64) + (x as u64) * (BYTES_PER_PIXEL as u64);
        if let Some(ptr) = self.fb_region.ptr_at(offset) {
            // SAFETY: Bounds checked above, ptr is within the mapped FB region.
            unsafe { ptr::write_volatile(ptr.cast::<u32>(), color) };
        }
    }

    fn fill_rect(&self, x: u32, y: u32, width: u32, height: u32, color: u32) {
        let x_end = (x + width).min(self.info.width);
        let y_end = (y + height).min(self.info.height);
        let base = self.fb_region.virt_base().as_u64();
        let pitch = self.info.pitch as u64;

        for row in y..y_end {
            let row_offset = (row as u64) * pitch + (x as u64) * (BYTES_PER_PIXEL as u64);
            let row_ptr = (base + row_offset) as *mut u32;
            for col in 0..(x_end - x) {
                // SAFETY: Bounds clamped above, writing within the mapped FB.
                unsafe { ptr::write_volatile(row_ptr.add(col as usize), color) };
            }
        }
    }

    unsafe fn copy_within(&self, src_offset: u64, dst_offset: u64, count: usize) {
        let base = self.fb_region.virt_base().as_u64() as *mut u8;
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
        let base = self.fb_region.virt_base().as_u64() as *mut u8;
        // SAFETY: Caller guarantees offset and count are within FB bounds.
        unsafe {
            ptr::write_bytes(base.add(offset as usize), 0, count);
        }
    }
}

// ---------------------------------------------------------------------------
// Global state + PCI registration
// ---------------------------------------------------------------------------

/// PCI device ID table for Bochs VGA.
#[cfg(target_os = "none")]
static ID_TABLE: [PciDeviceId; 1] = [PciDeviceId::new(BGA_VENDOR_ID, BGA_DEVICE_ID)];

#[cfg(target_os = "none")]
hadron_kernel::pci_driver_entry!(
    BOCHS_VGA_DRIVER,
    hadron_kernel::driver_api::registration::PciDriverEntry {
        name: "bochs-vga",
        id_table: &ID_TABLE,
        probe: bochs_vga_probe,
    }
);

/// PCI probe function for the Bochs VGA adapter.
#[cfg(target_os = "none")]
fn bochs_vga_probe(
    info: &PciDeviceInfo,
    services: &'static dyn KernelServices,
) -> Result<(), DriverError> {
    let vga = BochsVga::init(info, services, DEFAULT_WIDTH, DEFAULT_HEIGHT, DEFAULT_BPP)?;

    // Register framebuffer with the kernel's device registry.
    let vga_arc = alloc::sync::Arc::new(vga);
    services.register_framebuffer("bochs-vga", vga_arc);

    hadron_kernel::kinfo!("bochs-vga: driver initialized successfully");
    Ok(())
}
