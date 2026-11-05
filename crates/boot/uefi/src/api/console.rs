use core::fmt;
use core::marker::PhantomData;

use crate::EfiStatus;
use crate::protocol::simple_text::{self, SimpleTextOutputMode, SimpleTextOutputProtocol};

/// Safe wrapper around a UEFI Simple Text Output Protocol (console).
///
/// All methods use `&self` (interior mutability through the firmware pointer).
pub struct Console<'st> {
    raw: *mut SimpleTextOutputProtocol,
    _lifetime: PhantomData<&'st ()>,
}

impl Console<'_> {
    pub(crate) fn new(raw: *mut SimpleTextOutputProtocol) -> Self {
        Self {
            raw,
            _lifetime: PhantomData,
        }
    }

    /// Output a UTF-8 string to the console.
    ///
    /// Internally converts to UCS-2 in 128-`u16` stack-allocated chunks.
    /// Newlines (`\n`) are translated to `\r\n`.
    ///
    /// # Errors
    ///
    /// Returns the status from `OutputString` for the first chunk the firmware
    /// rejects, for example `EFI_DEVICE_ERROR`. Chunks before it have already been
    /// written.
    pub fn output_string(&self, s: &str) -> Result<(), EfiStatus> {
        const CHUNK: usize = 128;
        let mut buf = [0u16; CHUNK];
        let mut i = 0;

        for ch in s.chars() {
            if ch == '\n' {
                // Need space for \r + \n (2 chars) plus null terminator
                if i + 2 >= CHUNK {
                    buf[i] = 0;
                    let status = unsafe { ((*self.raw).output_string)(self.raw, buf.as_ptr()) };
                    status.to_result()?;
                    i = 0;
                }
                buf[i] = u16::from(b'\r');
                i += 1;
                buf[i] = u16::from(b'\n');
                i += 1;
            } else {
                let code = if (ch as u32) > 0xFFFF {
                    0xFFFD // replacement character for non-BMP
                } else {
                    ch as u16
                };
                // Need space for 1 char plus null terminator
                if i + 1 >= CHUNK {
                    buf[i] = 0;
                    let status = unsafe { ((*self.raw).output_string)(self.raw, buf.as_ptr()) };
                    status.to_result()?;
                    i = 0;
                }
                buf[i] = code;
                i += 1;
            }
        }

        // Flush remaining characters
        if i > 0 {
            buf[i] = 0;
            let status = unsafe { ((*self.raw).output_string)(self.raw, buf.as_ptr()) };
            status.to_result()?;
        }

        Ok(())
    }

    /// Clear the console screen.
    ///
    /// # Errors
    ///
    /// Returns the status from `ClearScreen` if it fails, for example `EFI_DEVICE_ERROR`.
    pub fn clear_screen(&self) -> Result<(), EfiStatus> {
        let status = unsafe { ((*self.raw).clear_screen)(self.raw) };
        status.to_result()
    }

    /// Set the text attribute (foreground and background colors).
    ///
    /// Use constants from [`simple_text::color`] for color values.
    ///
    /// # Errors
    ///
    /// Returns the status from `SetAttribute` if it fails, for example `EFI_DEVICE_ERROR`.
    pub fn set_attribute(&self, foreground: usize, background: usize) -> Result<(), EfiStatus> {
        let attr = simple_text::color::attribute(foreground, background);
        let status = unsafe { ((*self.raw).set_attribute)(self.raw, attr) };
        status.to_result()
    }

    /// Set the cursor position.
    ///
    /// # Errors
    ///
    /// Returns the status from `SetCursorPosition` if it fails, for example
    /// `EFI_UNSUPPORTED` when the position is outside the current mode.
    pub fn set_cursor_position(&self, column: usize, row: usize) -> Result<(), EfiStatus> {
        let status = unsafe { ((*self.raw).set_cursor_position)(self.raw, column, row) };
        status.to_result()
    }

    /// Reset the console output device.
    ///
    /// # Errors
    ///
    /// Returns the status from `Reset` if it fails, for example `EFI_DEVICE_ERROR`.
    pub fn reset(&self) -> Result<(), EfiStatus> {
        let status = unsafe { ((*self.raw).reset)(self.raw, false) };
        status.to_result()
    }

    /// Returns the current console mode information.
    #[must_use]
    pub fn mode(&self) -> &SimpleTextOutputMode {
        unsafe { &*(*self.raw).mode }
    }
}

impl fmt::Write for Console<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.output_string(s).map_err(|_| fmt::Error)
    }
}
