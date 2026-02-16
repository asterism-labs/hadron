//! UEFI Simple Text Output Protocol.
//!
//! This protocol is used to control text-based output devices. It supports
//! printing UCS-2 strings, controlling cursor position, and setting text colors.

use crate::EfiStatus;

/// The Simple Text Output Protocol.
///
/// Provides a basic interface for text-mode output to a console device.
#[repr(C)]
pub struct SimpleTextOutputProtocol {
    /// Resets the text output device hardware.
    pub reset: unsafe extern "efiapi" fn(
        this: *mut SimpleTextOutputProtocol,
        extended_verification: bool,
    ) -> EfiStatus,
    /// Writes a null-terminated UCS-2 string to the output device.
    pub output_string: unsafe extern "efiapi" fn(
        this: *mut SimpleTextOutputProtocol,
        string: *const u16,
    ) -> EfiStatus,
    /// Verifies that all characters in a UCS-2 string can be output to the target device.
    pub test_string: unsafe extern "efiapi" fn(
        this: *mut SimpleTextOutputProtocol,
        string: *const u16,
    ) -> EfiStatus,
    /// Returns information for an available text mode.
    pub query_mode: unsafe extern "efiapi" fn(
        this: *mut SimpleTextOutputProtocol,
        mode_number: usize,
        columns: *mut usize,
        rows: *mut usize,
    ) -> EfiStatus,
    /// Sets the output device to a specified mode.
    pub set_mode: unsafe extern "efiapi" fn(
        this: *mut SimpleTextOutputProtocol,
        mode_number: usize,
    ) -> EfiStatus,
    /// Sets the foreground and background colors for the `output_string` and `clear_screen`
    /// functions.
    pub set_attribute: unsafe extern "efiapi" fn(
        this: *mut SimpleTextOutputProtocol,
        attribute: usize,
    ) -> EfiStatus,
    /// Clears the output device display to the currently selected background color.
    pub clear_screen:
        unsafe extern "efiapi" fn(this: *mut SimpleTextOutputProtocol) -> EfiStatus,
    /// Sets the current coordinates of the cursor position.
    pub set_cursor_position: unsafe extern "efiapi" fn(
        this: *mut SimpleTextOutputProtocol,
        column: usize,
        row: usize,
    ) -> EfiStatus,
    /// Makes the cursor visible or invisible.
    pub enable_cursor: unsafe extern "efiapi" fn(
        this: *mut SimpleTextOutputProtocol,
        visible: bool,
    ) -> EfiStatus,
    /// Pointer to the current mode data.
    pub mode: *mut SimpleTextOutputMode,
}

impl SimpleTextOutputProtocol {
    /// Writes a null-terminated UCS-2 string to the output device.
    ///
    /// # Errors
    ///
    /// Returns `Err(EfiStatus)` if the device cannot output the string.
    ///
    /// # Safety
    ///
    /// The caller must ensure `self` points to a valid protocol instance and
    /// that `string` is a valid, null-terminated UCS-2 string.
    pub unsafe fn output_string(&mut self, string: *const u16) -> Result<(), EfiStatus> {
        let status = unsafe { (self.output_string)(self, string) };
        status.to_result()
    }

    /// Clears the screen.
    ///
    /// # Errors
    ///
    /// Returns `Err(EfiStatus)` if the screen cannot be cleared.
    ///
    /// # Safety
    ///
    /// The caller must ensure `self` points to a valid protocol instance.
    pub unsafe fn clear_screen(&mut self) -> Result<(), EfiStatus> {
        let status = unsafe { (self.clear_screen)(self) };
        status.to_result()
    }
}

/// Current mode information for the text output device.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SimpleTextOutputMode {
    /// The number of modes supported by `query_mode` and `set_mode`.
    pub max_mode: i32,
    /// The text mode of the output device.
    pub mode: i32,
    /// The current character output attribute.
    pub attribute: i32,
    /// The cursor's column.
    pub cursor_column: i32,
    /// The cursor's row.
    pub cursor_row: i32,
    /// Whether the cursor is currently visible.
    pub cursor_visible: bool,
}

/// Text color constants for the Simple Text Output Protocol.
///
/// These can be combined using the [`attribute`] function to create an attribute
/// value for `set_attribute`.
pub mod color {
    /// Black.
    pub const BLACK: usize = 0x00;
    /// Blue.
    pub const BLUE: usize = 0x01;
    /// Green.
    pub const GREEN: usize = 0x02;
    /// Cyan.
    pub const CYAN: usize = 0x03;
    /// Red.
    pub const RED: usize = 0x04;
    /// Magenta.
    pub const MAGENTA: usize = 0x05;
    /// Brown.
    pub const BROWN: usize = 0x06;
    /// Light gray.
    pub const LIGHT_GRAY: usize = 0x07;
    /// Dark gray (bright black).
    pub const DARK_GRAY: usize = 0x08;
    /// Light blue.
    pub const LIGHT_BLUE: usize = 0x09;
    /// Light green.
    pub const LIGHT_GREEN: usize = 0x0A;
    /// Light cyan.
    pub const LIGHT_CYAN: usize = 0x0B;
    /// Light red.
    pub const LIGHT_RED: usize = 0x0C;
    /// Light magenta.
    pub const LIGHT_MAGENTA: usize = 0x0D;
    /// Yellow.
    pub const YELLOW: usize = 0x0E;
    /// White.
    pub const WHITE: usize = 0x0F;

    /// Constructs a text attribute from foreground and background colors.
    ///
    /// Background colors must be in the range `0x00..=0x07` (i.e., the non-bright colors).
    #[must_use]
    pub const fn attribute(foreground: usize, background: usize) -> usize {
        (background << 4) | foreground
    }
}

// ── Compile-time layout assertions ──────────────────────────────────

const _: () = assert!(core::mem::size_of::<SimpleTextOutputMode>() == 24);

#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<SimpleTextOutputProtocol>() == 80);
