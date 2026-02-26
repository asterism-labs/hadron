//! Glyph rendering for the framebuffer console.
//!
//! Renders [`Cell`]s onto a [`Framebuffer`] using the embedded bitmap font.

use crate::driver_api::framebuffer::{Framebuffer, FramebufferInfo};
use crate::drivers::font_console::px16;

use super::cell::Cell;

/// Maximum glyph width in pixels (stack buffer size).
const MAX_GLYPH_WIDTH: usize = 32;

/// Renders a single cell at character position (`col`, `row`) onto the
/// framebuffer.
///
/// Builds each glyph scanline in a stack buffer (WB-cached), then bulk-copies
/// to the framebuffer via [`Framebuffer::write_scanline`]. This reduces per-cell
/// overhead from ~256 individual volatile writes to 16 bulk copies.
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

    let w = (glyph_width as usize).min(MAX_GLYPH_WIDTH);

    // Look up the glyph bitmap.
    let glyph_offset = px16::glyph_index(cell.ch as char)
        .or_else(|| px16::glyph_index(' '))
        .unwrap_or(0);
    let glyph = &px16::DATA[glyph_offset..][..px16::BYTES_PER_GLYPH];

    // The font is 1bpp with ceil(width/8) bytes per scanline, MSB-first.
    let row_bytes = (px16::WIDTH as usize).div_ceil(8);

    for dy in 0..glyph_height.min(px16::HEIGHT) {
        let mut buf = [bg; MAX_GLYPH_WIDTH];
        let scanline_start = dy as usize * row_bytes;

        for dx in 0..w.min(px16::WIDTH as usize) {
            let byte_idx = scanline_start + (dx / 8);
            let bit_idx = 7 - (dx as u32 % 8);
            let bit = (glyph[byte_idx] >> bit_idx) & 1;
            if bit != 0 {
                buf[dx] = fg;
            }
        }

        fb.write_scanline(x0, y0 + dy, &buf[..w]);
    }
}
