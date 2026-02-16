//! Framebuffer interface trait and associated types.

use hadron_core::addr::VirtAddr;

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
pub trait Framebuffer {
    /// Returns metadata about this framebuffer.
    fn info(&self) -> FramebufferInfo;

    /// Returns the virtual base address of the framebuffer memory.
    fn base_address(&self) -> VirtAddr;

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
}
