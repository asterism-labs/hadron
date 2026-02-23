//! Minimal 2D rasterizer for Hadron userspace.
//!
//! Operates on raw pixel buffers — no allocations needed. The [`Surface`] type
//! wraps a mutable `&[u32]` slice and provides pixel-level drawing primitives.
//! Format-agnostic: works with any 32-bit pixel layout. Use [`rgb`] or [`bgr`]
//! helpers for color packing.

#![no_std]

/// A drawable surface wrapping a mutable pixel buffer.
pub struct Surface<'a> {
    data: &'a mut [u32],
    width: u32,
    height: u32,
    stride: u32, // pixels per row (pitch / 4 for 32bpp)
}

impl<'a> Surface<'a> {
    /// Creates a new surface from a raw pixel buffer.
    ///
    /// `stride` is the number of u32 pixels per row (may differ from `width`
    /// if the framebuffer pitch includes padding).
    pub fn from_raw(data: &'a mut [u32], width: u32, height: u32, stride: u32) -> Self {
        Self {
            data,
            width,
            height,
            stride,
        }
    }

    /// Returns the surface width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the surface height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Sets a single pixel. Out-of-bounds coordinates are silently ignored.
    pub fn put_pixel(&mut self, x: u32, y: u32, color: u32) {
        if x < self.width && y < self.height {
            let idx = (y * self.stride + x) as usize;
            if idx < self.data.len() {
                self.data[idx] = color;
            }
        }
    }

    /// Fills the entire surface with a single color.
    pub fn fill(&mut self, color: u32) {
        for y in 0..self.height {
            let row_start = (y * self.stride) as usize;
            let row_end = row_start + self.width as usize;
            if row_end <= self.data.len() {
                self.data[row_start..row_end].fill(color);
            }
        }
    }

    /// Fills a rectangle at `(x, y)` with dimensions `w x h`.
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        let x_end = (x + w).min(self.width);
        let y_end = (y + h).min(self.height);

        for row in y..y_end {
            let row_start = (row * self.stride + x) as usize;
            let row_end = (row * self.stride + x_end) as usize;
            if row_end <= self.data.len() {
                self.data[row_start..row_end].fill(color);
            }
        }
    }

    /// Draws a horizontal line starting at `(x, y)` for `len` pixels.
    pub fn hline(&mut self, x: u32, y: u32, len: u32, color: u32) {
        if y >= self.height {
            return;
        }
        let x_end = (x + len).min(self.width);
        let row_start = (y * self.stride + x) as usize;
        let row_end = (y * self.stride + x_end) as usize;
        if row_end <= self.data.len() {
            self.data[row_start..row_end].fill(color);
        }
    }

    /// Draws a vertical line starting at `(x, y)` for `len` pixels.
    pub fn vline(&mut self, x: u32, y: u32, len: u32, color: u32) {
        if x >= self.width {
            return;
        }
        let y_end = (y + len).min(self.height);
        for row in y..y_end {
            let idx = (row * self.stride + x) as usize;
            if idx < self.data.len() {
                self.data[idx] = color;
            }
        }
    }

    /// Blits (copies) a source surface onto this surface at `(dst_x, dst_y)`.
    ///
    /// Negative destination coordinates clip the source. Pixels outside the
    /// destination bounds are silently skipped.
    pub fn blit(&mut self, src: &Surface, dst_x: i32, dst_y: i32) {
        for sy in 0..src.height {
            let dy = dst_y + sy as i32;
            if dy < 0 || dy >= self.height as i32 {
                continue;
            }
            for sx in 0..src.width {
                let dx = dst_x + sx as i32;
                if dx < 0 || dx >= self.width as i32 {
                    continue;
                }
                let src_idx = (sy * src.stride + sx) as usize;
                let dst_idx = (dy as u32 * self.stride + dx as u32) as usize;
                if src_idx < src.data.len() && dst_idx < self.data.len() {
                    self.data[dst_idx] = src.data[src_idx];
                }
            }
        }
    }
}

/// Pack RGB components into a 0x00RRGGBB pixel value.
#[inline]
pub fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

/// Pack RGB components into a 0x00BBGGRR pixel value (BGR byte order).
#[inline]
pub fn bgr(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(b) << 16) | (u32::from(g) << 8) | u32::from(r)
}
