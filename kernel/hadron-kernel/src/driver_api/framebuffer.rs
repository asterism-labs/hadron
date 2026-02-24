//! Framebuffer interface trait and associated types.

use crate::addr::{PhysAddr, VirtAddr};

/// Pixel format of a framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32-bit RGB (red at lowest byte offset).
    Rgb32,
    /// 32-bit BGR (blue at lowest byte offset).
    Bgr32,
    /// Arbitrary bitmask layout described by per-channel size and shift.
    Bitmask {
        /// Number of bits in the red channel.
        red_size: u8,
        /// Bit position of the red channel (from LSB).
        red_shift: u8,
        /// Number of bits in the green channel.
        green_size: u8,
        /// Bit position of the green channel (from LSB).
        green_shift: u8,
        /// Number of bits in the blue channel.
        blue_size: u8,
        /// Bit position of the blue channel (from LSB).
        blue_shift: u8,
    },
}

/// Metadata describing a framebuffer's dimensions and pixel layout.
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per scanline (may be larger than `width * bpp / 8` due to alignment).
    pub pitch: u32,
    /// Bits per pixel.
    pub bpp: u8,
    /// Pixel format.
    pub pixel_format: PixelFormat,
}

/// Interface trait for framebuffer devices.
///
/// Provides pixel-level access to a linear framebuffer. Methods take `&self`
/// because hardware I/O is inherently shared-state; callers use external
/// synchronization when needed.
pub trait Framebuffer: Send + Sync {
    /// Returns metadata about this framebuffer.
    fn info(&self) -> FramebufferInfo;

    /// Returns the virtual base address of the framebuffer memory.
    fn base_address(&self) -> VirtAddr;

    /// Returns the physical base address of the framebuffer memory.
    ///
    /// Required for userspace mmap via `/dev/fb0`.
    fn physical_base(&self) -> PhysAddr;

    /// Writes a pixel at the given coordinates.
    fn put_pixel(&self, x: u32, y: u32, color: u32);

    /// Fills a rectangle with the given color.
    ///
    /// Default implementation calls [`put_pixel`](Self::put_pixel) in a loop.
    fn fill_rect(&self, x: u32, y: u32, width: u32, height: u32, color: u32) {
        for row in y..y.saturating_add(height) {
            for col in x..x.saturating_add(width) {
                self.put_pixel(col, row, color);
            }
        }
    }

    /// Writes a horizontal span of pixels starting at (`x`, `y`).
    ///
    /// The default implementation falls back to [`put_pixel`](Self::put_pixel)
    /// in a loop. Drivers should override this with a bulk memory copy for
    /// better throughput on write-combine or uncacheable framebuffer memory.
    fn write_scanline(&self, x: u32, y: u32, pixels: &[u32]) {
        for (i, &color) in pixels.iter().enumerate() {
            self.put_pixel(x + i as u32, y, color);
        }
    }

    /// Copies `count` bytes within the framebuffer from `src_offset` to `dst_offset`.
    ///
    /// # Safety
    ///
    /// The caller must ensure both offset ranges are within the framebuffer bounds
    /// and that the regions are valid for the copy direction.
    unsafe fn copy_within(&self, src_offset: u64, dst_offset: u64, count: usize);

    /// Fills `count` bytes starting at `offset` with zeroes.
    ///
    /// # Safety
    ///
    /// The caller must ensure the range `[offset, offset + count)` is within
    /// the framebuffer bounds.
    unsafe fn fill_zero(&self, offset: u64, count: usize);

    /// Flushes a dirty rectangle to the display.
    ///
    /// For MMIO-backed framebuffers this is a no-op (writes are immediately
    /// visible). RAM-backed framebuffers (e.g. VirtIO GPU) override this to
    /// transfer the region to the host and request a display update.
    fn flush_rect(&self, _x: u32, _y: u32, _w: u32, _h: u32) {}

    /// Whether the framebuffer is backed by cacheable RAM.
    ///
    /// Returns `false` for MMIO-backed framebuffers (default) and `true` for
    /// RAM-backed framebuffers like VirtIO GPU. Used to decide whether mmap
    /// should use write-back or uncacheable mappings.
    fn is_ram_backed(&self) -> bool {
        false
    }
}
