//! UEFI Graphics Output Protocol (GOP).
//!
//! The Graphics Output Protocol provides a hardware abstraction for the video display.
//! It allows querying available video modes, setting a mode, and performing block transfers
//! (Blt) to and from the framebuffer.

use crate::EfiStatus;

/// The Graphics Output Protocol.
#[repr(C)]
pub struct GraphicsOutputProtocol {
    /// Returns information for an available graphics mode.
    pub query_mode: unsafe extern "efiapi" fn(
        this: *mut GraphicsOutputProtocol,
        mode_number: u32,
        size_of_info: *mut usize,
        info: *mut *mut GraphicsOutputModeInformation,
    ) -> EfiStatus,
    /// Sets the video device into a specified mode.
    pub set_mode:
        unsafe extern "efiapi" fn(this: *mut GraphicsOutputProtocol, mode_number: u32) -> EfiStatus,
    /// Performs a block transfer (Blt) operation.
    pub blt: unsafe extern "efiapi" fn(
        this: *mut GraphicsOutputProtocol,
        blt_buffer: *mut BltPixel,
        blt_operation: BltOperation,
        source_x: usize,
        source_y: usize,
        destination_x: usize,
        destination_y: usize,
        width: usize,
        height: usize,
        delta: usize,
    ) -> EfiStatus,
    /// Pointer to the current mode data.
    pub mode: *mut GraphicsOutputMode,
}

/// Current mode information for the graphics output device.
#[repr(C)]
#[derive(Debug)]
pub struct GraphicsOutputMode {
    /// The number of modes supported by `query_mode` and `set_mode`.
    pub max_mode: u32,
    /// Current mode of the graphics device.
    pub mode: u32,
    /// Pointer to mode information for the current mode.
    pub info: *mut GraphicsOutputModeInformation,
    /// Size of the `info` structure in bytes.
    pub size_of_info: usize,
    /// Base address of the graphics linear frame buffer.
    pub frame_buffer_base: u64,
    /// Size of the frame buffer in bytes.
    pub frame_buffer_size: usize,
}

/// Information about a specific graphics mode.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GraphicsOutputModeInformation {
    /// The version of this data structure (zero for UEFI 2.1+).
    pub version: u32,
    /// The horizontal resolution in pixels.
    pub horizontal_resolution: u32,
    /// The vertical resolution in pixels.
    pub vertical_resolution: u32,
    /// The pixel format of the physical frame buffer.
    pub pixel_format: PixelFormat,
    /// Valid only if `pixel_format` is `BitmaskPixel`.
    pub pixel_information: PixelBitmask,
    /// The number of pixels per scan line. This may be larger than `horizontal_resolution`
    /// due to padding.
    pub pixels_per_scan_line: u32,
}

/// Pixel format for the frame buffer.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// A pixel has 32 bits: Red, Green, Blue, Reserved (in byte order).
    RedGreenBlueReserved8BitPerColor = 0,
    /// A pixel has 32 bits: Blue, Green, Red, Reserved (in byte order).
    BlueGreenRedReserved8BitPerColor = 1,
    /// The pixel layout is defined by `pixel_information` bitmasks.
    BitmaskPixel = 2,
    /// The graphics mode does not support a physical frame buffer.
    BltOnly = 3,
}

/// Bitmask describing the pixel layout when `PixelFormat::BitmaskPixel` is used.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PixelBitmask {
    /// The bits indicating the red channel.
    pub red_mask: u32,
    /// The bits indicating the green channel.
    pub green_mask: u32,
    /// The bits indicating the blue channel.
    pub blue_mask: u32,
    /// The reserved bits.
    pub reserved_mask: u32,
}

/// A pixel for BLT operations.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BltPixel {
    /// Blue channel value.
    pub blue: u8,
    /// Green channel value.
    pub green: u8,
    /// Red channel value.
    pub red: u8,
    /// Reserved (must be zero).
    pub reserved: u8,
}

/// Block transfer operations for the GOP.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BltOperation {
    /// Write data from the BLT buffer directly to every pixel of the video display.
    VideoFill = 0,
    /// Read data from the video display to the BLT buffer.
    VideoToBltBuffer = 1,
    /// Write data from the BLT buffer to the video display.
    BltBufferToVideo = 2,
    /// Copy from the video display to the video display.
    VideoToVideo = 3,
}

// ── Compile-time layout assertions ──────────────────────────────────

const _: () = {
    assert!(core::mem::size_of::<PixelBitmask>() == 16);
    assert!(core::mem::size_of::<BltPixel>() == 4);
    assert!(core::mem::size_of::<GraphicsOutputModeInformation>() == 36);
};

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<GraphicsOutputProtocol>() == 32);
    assert!(core::mem::size_of::<GraphicsOutputMode>() == 40);
};
