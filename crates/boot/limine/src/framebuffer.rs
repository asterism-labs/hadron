//! Framebuffer structures and iterators.
//!
//! This module provides types for working with graphical framebuffers provided by
//! the Limine bootloader. Framebuffers allow direct pixel access for graphics output.
//!
//! # Overview
//!
//! The main types in this module are:
//! - [`Framebuffer`] - A safe wrapper around framebuffer information
//! - [`VideoMode`] - Describes a video mode's resolution and pixel format
//! - [`MemoryModel`] - Specifies the pixel format (currently only RGB is supported)
//! - [`FramebufferIter`] - Iterator over multiple framebuffers (for multi-monitor setups)
//!
//! # Example
//!
//! ```no_run
//! use limine::FramebufferRequest;
//!
//! #[used]
//! #[link_section = ".requests"]
//! static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();
//!
//! fn draw_pixel(x: u64, y: u64, color: u32) {
//!     if let Some(fb_response) = FRAMEBUFFER_REQUEST.response() {
//!         for framebuffer in fb_response.framebuffers() {
//!             let mode = &framebuffer.default_mode;
//!             if x < mode.width && y < mode.height {
//!                 let offset = (y * mode.pitch + x * (mode.bpp as u64 / 8)) as isize;
//!                 unsafe {
//!                     let fb_ptr = framebuffer.addr.as_ptr().offset(offset) as *mut u32;
//!                     *fb_ptr = color;
//!                 }
//!             }
//!         }
//!     }
//! }
//! ```

use core::{ffi::c_void, ptr::NonNull};

/// Memory model of the framebuffer.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryModel {
    /// RGB pixel format where each pixel has separate red, green, and blue channels.
    RGB = 1,
}

/// A video mode supported by the framebuffer.
#[repr(C)]
pub struct VideoMode {
    /// Number of bytes per scanline.
    pub pitch: u64,
    /// Width in pixels.
    pub width: u64,
    /// Height in pixels.
    pub height: u64,
    /// Bits per pixel.
    pub bpp: u16,
    /// Memory model describing the pixel format.
    pub memory_model: MemoryModel,
    /// Red mask size (in bits).
    pub red_mask_size: u8,
    /// Red mask shift (from LSB).
    pub red_mask_shift: u8,
    /// Green mask size (in bits).
    pub green_mask_size: u8,
    /// Green mask shift (from LSB).
    pub green_mask_shift: u8,
    /// Blue mask size (in bits).
    pub blue_mask_size: u8,
    /// Blue mask shift (from LSB).
    pub blue_mask_shift: u8,
}

/// Raw framebuffer structure for revision 0.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawFramebufferV0 {
    /// Base address of the framebuffer memory region.
    pub address: NonNull<c_void>,
    /// Width in pixels.
    pub width: u64,
    /// Height in pixels.
    pub height: u64,
    /// Number of bytes per scanline.
    pub pitch: u64,
    /// Bits per pixel.
    pub bpp: u16,
    /// Memory model describing the pixel format.
    pub memory_model: MemoryModel,
    /// Red mask size (in bits).
    pub red_mask_size: u8,
    /// Red mask shift (from LSB).
    pub red_mask_shift: u8,
    /// Green mask size (in bits).
    pub green_mask_size: u8,
    /// Green mask shift (from LSB).
    pub green_mask_shift: u8,
    /// Blue mask size (in bits).
    pub blue_mask_size: u8,
    /// Blue mask shift (from LSB).
    pub blue_mask_shift: u8,
    _unused: [u8; 7],
    /// Size of the EDID data in bytes.
    pub edid_size: u64,
    /// Pointer to the EDID data, or null if not available.
    pub edid: *const c_void,
}

impl RawFramebufferV0 {
    /// Converts this raw framebuffer to a safe [`Framebuffer`] wrapper.
    #[must_use]
    pub fn to_framebuffer(&self) -> Framebuffer {
        let default_mode = VideoMode {
            pitch: self.pitch,
            width: self.width,
            height: self.height,
            bpp: self.bpp,
            memory_model: self.memory_model,
            red_mask_size: self.red_mask_size,
            red_mask_shift: self.red_mask_shift,
            green_mask_size: self.green_mask_size,
            green_mask_shift: self.green_mask_shift,
            blue_mask_size: self.blue_mask_size,
            blue_mask_shift: self.blue_mask_shift,
        };

        Framebuffer {
            addr: self.address.cast(),
            default_mode,
            modes: &[],
        }
    }
}

/// Raw framebuffer structure for revision 1.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawFramebufferV1 {
    /// The revision 0 framebuffer data.
    pub inner: RawFramebufferV0,
    /// Number of video modes available.
    pub mode_count: u64,
    /// Pointer to the array of supported video modes.
    pub modes: NonNull<&'static VideoMode>,
}

/// A raw framebuffer union that can represent different revisions.
#[repr(C)]
pub union RawFramebuffer {
    /// Revision 0 framebuffer layout.
    pub v0: RawFramebufferV0,
    /// Revision 1 framebuffer layout (extends v0 with video modes).
    pub v1: RawFramebufferV1,
}

impl RawFramebuffer {
    /// Convert a raw framebuffer pointer to a safe Framebuffer struct based on the revision.
    ///
    /// # Safety
    /// The caller must ensure that `fb` points to a valid `RawFramebuffer` structure
    /// matching the specified `revision`.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0 or 1.
    #[must_use]
    pub unsafe fn to_fb(revision: u64, fb: NonNull<RawFramebuffer>) -> Framebuffer {
        // SAFETY: The caller guarantees that `fb` points to a valid RawFramebuffer
        // and that `revision` matches the actual layout of the data.
        unsafe {
            match revision {
                0 => fb.as_ref().v0.to_framebuffer(),
                1 => {
                    let mut framebuffer = fb.as_ref().v1.inner.to_framebuffer();
                    // SAFETY: For revision 1, the bootloader provides a valid pointer
                    // to `mode_count` video mode references.
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "pixel format fields fit in usize"
                    )]
                    let modes = core::slice::from_raw_parts(
                        fb.as_ref().v1.modes.as_ptr(),
                        fb.as_ref().v1.mode_count as usize,
                    );
                    framebuffer.modes = modes;
                    framebuffer
                }
                _ => panic!("Unsupported framebuffer revision: {}", revision),
            }
        }
    }
}

/// An iterator over framebuffers.
pub struct FramebufferIter<'a> {
    revision: u64,
    index: usize,
    framebuffers: &'a [NonNull<RawFramebuffer>],
}

impl FramebufferIter<'_> {
    /// Create a new framebuffer iterator.
    pub(crate) fn new(
        revision: u64,
        count: usize,
        framebuffers: NonNull<NonNull<RawFramebuffer>>,
    ) -> Self {
        // SAFETY: The bootloader provides a valid pointer to an array of `count`
        // NonNull<RawFramebuffer> pointers.
        let framebuffers = unsafe { core::slice::from_raw_parts(framebuffers.as_ptr(), count) };
        Self {
            revision,
            index: 0,
            framebuffers,
        }
    }
}

impl Iterator for FramebufferIter<'_> {
    type Item = Framebuffer;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.framebuffers.len() {
            return None;
        }

        // SAFETY: Each entry in the framebuffers slice is a valid NonNull pointer to a
        // RawFramebuffer matching the specified revision, as provided by the bootloader.
        let fb = unsafe { RawFramebuffer::to_fb(self.revision, self.framebuffers[self.index]) };
        self.index += 1;
        Some(fb)
    }
}

/// A wrapped 'safe' and ergonomic framebuffer structure.
pub struct Framebuffer {
    /// Base address of the framebuffer memory.
    pub addr: NonNull<u8>,
    /// The default (current) video mode of this framebuffer.
    pub default_mode: VideoMode,
    /// Available video modes that the framebuffer supports.
    pub modes: &'static [&'static VideoMode],
}
