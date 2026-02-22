//! Glyph rendering for the framebuffer console.
//!
//! Renders [`Cell`]s onto a [`Framebuffer`] using the embedded bitmap font.

use crate::driver_api::framebuffer::{Framebuffer, FramebufferInfo};
use crate::drivers::font_console::px16;

use super::cell::Cell;

/// Renders a single cell at character position (`col`, `row`) onto the
/// framebuffer.
///
/// 1. Fills the cell background via [`Framebuffer::fill_rect`].
/// 2. Draws the foreground glyph pixel-by-pixel from the 1bpp font data.
pub fn render_cell(
    fb: &dyn Framebuffer,
    info: &FramebufferInfo,
    col: u32,
    row: u32,
    cell: &Cell,
    glyph_width: u32,
    glyph_height: u32,
) {
    let x0 = col * glyph_width;
    let y0 = row * glyph_height;

    // Clamp to framebuffer bounds.
    if x0 >= info.width || y0 >= info.height {
        return;
    }

    let fg = cell.fg.to_bgr32(true);
    let bg = cell.bg.to_bgr32(false);

    // Fill the entire cell with the background color.
    fb.fill_rect(x0, y0, glyph_width, glyph_height, bg);

    // Look up the glyph bitmap.
    let glyph_offset = px16::glyph_index(cell.ch as char)
        .or_else(|| px16::glyph_index(' '))
        .unwrap_or(0);
    let glyph = &px16::DATA[glyph_offset..][..px16::BYTES_PER_GLYPH];

    // The font is 1bpp with ceil(width/8) bytes per scanline, MSB-first.
    let row_bytes = (px16::WIDTH as usize).div_ceil(8);

    for dy in 0..glyph_height.min(px16::HEIGHT) {
        let scanline_start = dy as usize * row_bytes;

        for dx in 0..glyph_width.min(px16::WIDTH) {
            let byte_idx = scanline_start + (dx as usize / 8);
            let bit_idx = 7 - (dx % 8);
            let bit = (glyph[byte_idx] >> bit_idx) & 1;

            if bit != 0 {
                fb.put_pixel(x0 + dx, y0 + dy, fg);
            }
        }
    }
}
