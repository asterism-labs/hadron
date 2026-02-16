//! UEFI Simple Text Input Protocol.
//!
//! This protocol is used to obtain input from the console. It provides a basic
//! keystroke interface.

use crate::{EfiEvent, EfiStatus};

/// The Simple Text Input Protocol.
///
/// Provides a basic interface for reading keystrokes from a console input device.
#[repr(C)]
pub struct SimpleTextInputProtocol {
    /// Resets the input device hardware.
    pub reset: unsafe extern "efiapi" fn(
        this: *mut SimpleTextInputProtocol,
        extended_verification: bool,
    ) -> EfiStatus,
    /// Reads the next keystroke from the input device.
    pub read_key_stroke: unsafe extern "efiapi" fn(
        this: *mut SimpleTextInputProtocol,
        key: *mut InputKey,
    ) -> EfiStatus,
    /// Event to wait for a keystroke.
    pub wait_for_key: EfiEvent,
}

/// Describes a keystroke.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputKey {
    /// The scan code for special keys (arrows, function keys, etc.).
    /// Zero if the key produces a Unicode character.
    pub scan_code: u16,
    /// The Unicode character for the key. Zero if the key is a special key.
    pub unicode_char: u16,
}

/// Scan code constants for special keys.
pub mod scan_code {
    /// Null scan code (key has a Unicode character instead).
    pub const NULL: u16 = 0x00;
    /// Up arrow key.
    pub const UP: u16 = 0x01;
    /// Down arrow key.
    pub const DOWN: u16 = 0x02;
    /// Right arrow key.
    pub const RIGHT: u16 = 0x03;
    /// Left arrow key.
    pub const LEFT: u16 = 0x04;
    /// Home key.
    pub const HOME: u16 = 0x05;
    /// End key.
    pub const END: u16 = 0x06;
    /// Insert key.
    pub const INSERT: u16 = 0x07;
    /// Delete key.
    pub const DELETE: u16 = 0x08;
    /// Page Up key.
    pub const PAGE_UP: u16 = 0x09;
    /// Page Down key.
    pub const PAGE_DOWN: u16 = 0x0A;
    /// Function key F1.
    pub const F1: u16 = 0x0B;
    /// Function key F2.
    pub const F2: u16 = 0x0C;
    /// Function key F3.
    pub const F3: u16 = 0x0D;
    /// Function key F4.
    pub const F4: u16 = 0x0E;
    /// Function key F5.
    pub const F5: u16 = 0x0F;
    /// Function key F6.
    pub const F6: u16 = 0x10;
    /// Function key F7.
    pub const F7: u16 = 0x11;
    /// Function key F8.
    pub const F8: u16 = 0x12;
    /// Function key F9.
    pub const F9: u16 = 0x13;
    /// Function key F10.
    pub const F10: u16 = 0x14;
    /// Escape key.
    pub const ESC: u16 = 0x17;
}

// ── Compile-time layout assertions ──────────────────────────────────

const _: () = assert!(core::mem::size_of::<InputKey>() == 4);

#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<SimpleTextInputProtocol>() == 24);
