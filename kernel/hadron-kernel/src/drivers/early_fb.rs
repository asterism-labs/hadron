//! Early framebuffer text console driver.
//!
//! Provides immediate visual output during boot by rendering text directly to a
//! linear framebuffer using an embedded 8x16 VGA bitmap font. Only supports
//! 32bpp framebuffers.

use core::fmt;
use core::ptr;

use hadron_core::addr::VirtAddr;

use crate::boot::{FramebufferInfo, PixelFormat};
use crate::drivers::font_8x16::VGA_FONT_8X16;
use crate::sync::SpinLock;

/// Reference to the embedded VGA 8x16 font data for use by other sinks.
pub(crate) static VGA_FONT_8X16_REF: &[u8] = &VGA_FONT_8X16;

/// Glyph width in pixels.
pub(crate) const GLYPH_WIDTH: u32 = 8;
/// Glyph height in pixels.
pub(crate) const GLYPH_HEIGHT: u32 = 16;

/// Text console cursor position.
pub(crate) struct CursorState {
    /// Current column.
    pub(crate) col: u32,
    /// Current row.
    pub(crate) row: u32,
}

pub(crate) static CURSOR: SpinLock<CursorState> = SpinLock::new(CursorState { col: 0, row: 0 });

/// A simple text console backed by a linear framebuffer.
///
/// All fields are scalar `Copy` types so the struct itself is `Copy`. Mutable
/// cursor state lives in the separate `CURSOR` static.
#[derive(Debug, Clone, Copy)]
pub struct EarlyFramebuffer {
    address: VirtAddr,
    width: u32,
    height: u32,
    pitch: u32,
    bytes_per_pixel: u32,
    cols: u32,
    rows: u32,
    fg_color: u32,
    bg_color: u32,
}

impl EarlyFramebuffer {
    /// Creates an `EarlyFramebuffer` from a [`FramebufferInfo`].
    ///
    /// Returns `None` if the framebuffer is not 32bpp.
    pub fn new(info: &FramebufferInfo) -> Option<Self> {
        if info.bpp != 32 {
            return None;
        }

        let cols = info.width / GLYPH_WIDTH;
        let rows = info.height / GLYPH_HEIGHT;

        // Pre-compute foreground (light grey) and background (black) colors in
        // the framebuffer's native pixel format.
        let (fg_color, bg_color) = match info.pixel_format {
            PixelFormat::Rgb32 => {
                // R at byte 0 (lowest), G at byte 1, B at byte 2
                let fg = 0x00_AA_AA_AA; // 0x00BBGGRR layout in memory
                let bg = 0x00_00_00_00;
                (fg, bg)
            }
            PixelFormat::Bgr32 => {
                // B at byte 0 (lowest), G at byte 1, R at byte 2
                let fg = 0x00_AA_AA_AA;
                let bg = 0x00_00_00_00;
                (fg, bg)
            }
            PixelFormat::Bitmask {
                red_size,
                red_shift,
                green_size,
                green_shift,
                blue_size,
                blue_shift,
            } => {
                let pack = |intensity: u32, size: u8, shift: u8| -> u32 {
                    let max = (1u32 << size) - 1;
                    let scaled = (intensity * max) / 255;
                    scaled << shift
                };
                let fg = pack(0xAA, red_size, red_shift)
                    | pack(0xAA, green_size, green_shift)
                    | pack(0xAA, blue_size, blue_shift);
                let bg = 0u32;
                (fg, bg)
            }
        };

        Some(Self {
            address: info.address,
            width: info.width,
            height: info.height,
            pitch: info.pitch,
            bytes_per_pixel: 4,
            cols,
            rows,
            fg_color,
            bg_color,
        })
    }

    fn put_pixel(&self, x: u32, y: u32, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = (y as u64) * (self.pitch as u64) + (x as u64) * (self.bytes_per_pixel as u64);
        let ptr = (self.address + offset).as_mut_ptr::<u32>();
        unsafe { ptr::write_volatile(ptr, color) };
    }

    fn draw_glyph(&self, col: u32, row: u32, ch: u8) {
        let glyph = &VGA_FONT_8X16[(ch as usize) * 16..][..16];
        let x0 = col * GLYPH_WIDTH;
        let y0 = row * GLYPH_HEIGHT;

        for (dy, &scanline) in glyph.iter().enumerate() {
            for dx in 0..GLYPH_WIDTH {
                let bit = (scanline >> (7 - dx)) & 1;
                let color = if bit != 0 {
                    self.fg_color
                } else {
                    self.bg_color
                };
                self.put_pixel(x0 + dx, y0 + dy as u32, color);
            }
        }
    }

    fn scroll_up(&self) {
        if self.rows <= 1 {
            return;
        }

        let row_bytes = self.pitch as usize * GLYPH_HEIGHT as usize;
        let total_rows = self.rows as usize;
        let base = self.address.as_mut_ptr::<u8>();
        let src = unsafe { base.add(row_bytes) } as *const u8;
        let dst = base;
        let count = row_bytes * (total_rows - 1);

        unsafe {
            ptr::copy(src, dst, count);
        }

        // Clear the last row.
        let last_row_start = unsafe { base.add(row_bytes * (total_rows - 1)) };
        unsafe {
            ptr::write_bytes(last_row_start, 0, row_bytes);
        }
    }

    pub(crate) fn write_byte_internal(&self, byte: u8, cursor: &mut CursorState) {
        match byte {
            b'\n' => {
                cursor.col = 0;
                cursor.row += 1;
            }
            b'\r' => {
                cursor.col = 0;
            }
            b'\t' => {
                let next = (cursor.col + 4) & !3;
                cursor.col = next;
                if cursor.col >= self.cols {
                    cursor.col = 0;
                    cursor.row += 1;
                }
            }
            byte => {
                if cursor.col >= self.cols {
                    cursor.col = 0;
                    cursor.row += 1;
                }
                if cursor.row >= self.rows {
                    self.scroll_up();
                    cursor.row = self.rows - 1;
                }
                self.draw_glyph(cursor.col, cursor.row, byte);
                cursor.col += 1;
            }
        }

        if cursor.row >= self.rows {
            self.scroll_up();
            cursor.row = self.rows - 1;
        }
    }
}

impl fmt::Write for EarlyFramebuffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut cursor = CURSOR.lock();
        for byte in s.bytes() {
            self.write_byte_internal(byte, &mut cursor);
        }
        Ok(())
    }
}

// Font data lives in drivers/font_8x16.rs (VGA_FONT_8X16, imported above).
