//! Parameters the bootloader used to set up its Flanterm framebuffer terminals.
//!
//! With these, the kernel can set up [Flanterm](https://github.com/Mintsuki/Flanterm) to
//! look the same as the bootloader's terminal: same wallpaper, colours, font, margin and
//! rotation. Request them with
//! [`FlantermFbInitParamsRequest`](crate::FlantermFbInitParamsRequest).

/// Display rotation of a Flanterm terminal.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlantermRotation(pub u64);

impl FlantermRotation {
    /// No rotation.
    pub const ROTATE_0: Self = Self(0);
    /// Rotated 90 degrees.
    pub const ROTATE_90: Self = Self(1);
    /// Rotated 180 degrees.
    pub const ROTATE_180: Self = Self(2);
    /// Rotated 270 degrees.
    pub const ROTATE_270: Self = Self(3);
}

/// Flanterm setup parameters for one framebuffer.
///
/// All pointers are HHDM virtual addresses. An entry for a framebuffer that cannot hold a
/// Flanterm terminal is all zero.
#[repr(C)]
pub struct FlantermFbInitParams {
    canvas: *const u32,
    /// Size of the background canvas in bytes, or 0 if no wallpaper is configured.
    pub canvas_size: u64,
    /// The 8 standard ANSI colours: black, red, green, brown, blue, magenta, cyan, grey.
    pub ansi_colours: [u32; 8],
    /// The 8 bright ANSI colours.
    pub ansi_bright_colours: [u32; 8],
    /// Default background colour.
    pub default_bg: u32,
    /// Default foreground colour.
    pub default_fg: u32,
    /// Default bright background colour.
    pub default_bg_bright: u32,
    /// Default bright foreground colour.
    pub default_fg_bright: u32,
    font: *const u8,
    /// Glyph width in pixels. Always 8 for VGA fonts.
    pub font_width: u64,
    /// Glyph height in pixels.
    pub font_height: u64,
    /// Extra horizontal space between glyphs, in pixels.
    pub font_spacing: u64,
    /// Horizontal font scale factor.
    pub font_scale_x: u64,
    /// Vertical font scale factor.
    pub font_scale_y: u64,
    /// Terminal margin from the screen edge, in pixels.
    pub margin: u64,
    /// Display rotation.
    pub rotation: FlantermRotation,
}

impl FlantermFbInitParams {
    /// Returns the pre-rendered background canvas, or `None` if no wallpaper is configured.
    ///
    /// The canvas holds 32-bit pixels in the framebuffer's pixel format, laid out at the
    /// framebuffer's width and height.
    #[must_use]
    pub fn canvas(&self) -> Option<&[u32]> {
        if self.canvas.is_null() {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Limine protocol fields fit in target width"
        )]
        let len = self.canvas_size as usize / size_of::<u32>();
        // SAFETY: A non-null canvas points to `canvas_size` bytes of 32-bit pixels that
        // stay valid for the lifetime of the kernel.
        Some(unsafe { core::slice::from_raw_parts(self.canvas, len) })
    }

    /// Returns the VGA-style bitmap font with 256 glyphs, or `None` if this entry is
    /// zeroed.
    ///
    /// This is the font the bootloader used, including its built-in default font. Its
    /// size is `font_width * font_height * 256 / 8` bytes.
    #[must_use]
    pub fn font(&self) -> Option<&[u8]> {
        if self.font.is_null() {
            return None;
        }
        let len = usize::try_from(self.font_width * self.font_height * 256 / 8).ok()?;
        // SAFETY: A non-null font points to `font_width * font_height * 256 / 8` bytes
        // that stay valid for the lifetime of the kernel.
        Some(unsafe { core::slice::from_raw_parts(self.font, len) })
    }
}

#[cfg(test)]
mod tests {
    use core::mem::offset_of;

    use super::*;

    #[test]
    fn layout_matches_spec() {
        assert_eq!(offset_of!(FlantermFbInitParams, canvas_size), 8);
        assert_eq!(offset_of!(FlantermFbInitParams, ansi_colours), 16);
        assert_eq!(offset_of!(FlantermFbInitParams, ansi_bright_colours), 48);
        assert_eq!(offset_of!(FlantermFbInitParams, default_bg), 80);
        assert_eq!(offset_of!(FlantermFbInitParams, default_fg_bright), 92);
        assert_eq!(offset_of!(FlantermFbInitParams, font), 96);
        assert_eq!(offset_of!(FlantermFbInitParams, font_width), 104);
        assert_eq!(offset_of!(FlantermFbInitParams, margin), 144);
        assert_eq!(offset_of!(FlantermFbInitParams, rotation), 152);
        assert_eq!(size_of::<FlantermFbInitParams>(), 160);
    }

    #[test]
    fn rotations_match_spec() {
        assert_eq!(FlantermRotation::ROTATE_0.0, 0);
        assert_eq!(FlantermRotation::ROTATE_90.0, 1);
        assert_eq!(FlantermRotation::ROTATE_180.0, 2);
        assert_eq!(FlantermRotation::ROTATE_270.0, 3);
    }
}
