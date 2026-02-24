//! Grid-to-surface rendering.
//!
//! Iterates over the terminal [`Grid`] and draws each cell's character glyph
//! onto a [`Surface`] using the 8x16 bitmap font from `lepton-gfx`.

use lepton_gfx::Surface;
use lepton_gfx::font;

use crate::grid::Grid;

/// Render the entire grid onto a surface.
pub fn render_grid(surface: &mut Surface<'_>, grid: &Grid) {
    for row in 0..grid.rows {
        for col in 0..grid.cols {
            let cell = &grid.cells[row * grid.cols + col];
            let px = col as u32 * font::WIDTH;
            let py = row as u32 * font::HEIGHT;
            surface.draw_char(px, py, cell.ch as char, cell.fg, cell.bg);
        }
    }

    // Draw a block cursor at the current position.
    let cx = grid.cursor_col as u32 * font::WIDTH;
    let cy = grid.cursor_row as u32 * font::HEIGHT;
    // Draw an underscore-style cursor (bottom 2 rows of the cell).
    let cursor_color = 0x00CC_CCCC;
    for dy in (font::HEIGHT - 2)..font::HEIGHT {
        for dx in 0..font::WIDTH {
            surface.put_pixel(cx + dx, cy + dy, cursor_color);
        }
    }
}
