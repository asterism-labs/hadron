//! Intel 8042 PS/2 controller driver.
//!
//! Provides a [`I8042`] type for the i8042 PS/2 keyboard/mouse controller.
//! Handles controller initialization, scancode-to-keycode translation (Set 1),
//! and 3-byte mouse packet parsing.

use core::fmt;

use bitflags::bitflags;
#[cfg(target_arch = "x86_64")]
use hadron_core::arch::x86_64::Port;
use hadron_driver_api::input::KeyCode;

// ---------------------------------------------------------------------------
// I/O ports (x86_64 only)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
/// Data port (read: output buffer, write: input buffer).
const DATA_PORT: u16 = 0x60;
#[cfg(target_arch = "x86_64")]
/// Status register (read) / command register (write).
const STATUS_CMD_PORT: u16 = 0x64;

// ---------------------------------------------------------------------------
// Controller commands (x86_64 only)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
/// Command bytes sent to the command port (0x64).
mod cmd {
    /// Read controller configuration byte.
    pub const READ_CONFIG: u8 = 0x20;
    /// Write controller configuration byte.
    pub const WRITE_CONFIG: u8 = 0x60;
    /// Disable second PS/2 port (mouse).
    pub const DISABLE_PORT2: u8 = 0xA7;
    /// Enable second PS/2 port (mouse).
    pub const ENABLE_PORT2: u8 = 0xA8;
    /// Controller self-test.
    pub const SELF_TEST: u8 = 0xAA;
    /// Disable first PS/2 port (keyboard).
    pub const DISABLE_PORT1: u8 = 0xAD;
    /// Enable first PS/2 port (keyboard).
    pub const ENABLE_PORT1: u8 = 0xAE;
}

#[cfg(target_arch = "x86_64")]
/// Expected self-test response byte.
const SELF_TEST_OK: u8 = 0x55;

#[cfg(target_arch = "x86_64")]
/// Device reset command (sent to data port).
const DEVICE_RESET: u8 = 0xFF;
#[cfg(target_arch = "x86_64")]
/// Acknowledge byte from PS/2 devices.
const ACK: u8 = 0xFA;
#[cfg(target_arch = "x86_64")]
/// Device self-test passed.
const DEVICE_SELF_TEST_OK: u8 = 0xAA;

// ---------------------------------------------------------------------------
// Bitflag types
// ---------------------------------------------------------------------------

bitflags! {
    /// Status register bits (read from port 0x64).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StatusReg: u8 {
        /// Output buffer full (data available to read).
        const OUTPUT_FULL   = 1 << 0;
        /// Input buffer full (controller busy, don't write).
        const INPUT_FULL    = 1 << 1;
        /// System flag (POST passed).
        const SYSTEM_FLAG   = 1 << 2;
        /// Data written to port 0x60 is command (0) or data (1).
        const COMMAND_DATA  = 1 << 3;
        /// Data from mouse port (vs keyboard).
        const MOUSE_DATA    = 1 << 5;
        /// Timeout error.
        const TIMEOUT_ERROR = 1 << 6;
        /// Parity error.
        const PARITY_ERROR  = 1 << 7;
    }
}

bitflags! {
    /// Controller configuration byte bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ConfigByte: u8 {
        /// Enable port 1 (keyboard) interrupt (IRQ 1).
        const PORT1_IRQ             = 1 << 0;
        /// Enable port 2 (mouse) interrupt (IRQ 12).
        const PORT2_IRQ             = 1 << 1;
        /// Disable port 1 clock.
        const PORT1_CLOCK_DISABLE   = 1 << 4;
        /// Disable port 2 clock.
        const PORT2_CLOCK_DISABLE   = 1 << 5;
        /// Enable scancode translation for port 1.
        const PORT1_TRANSLATION     = 1 << 6;
    }
}

// ---------------------------------------------------------------------------
// InitError
// ---------------------------------------------------------------------------

/// Error returned when i8042 initialization fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    /// Controller self-test failed.
    SelfTestFailed,
    /// Timeout waiting for controller.
    Timeout,
    /// Keyboard reset failed.
    KeyboardResetFailed,
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelfTestFailed => f.write_str("i8042 controller self-test failed"),
            Self::Timeout => f.write_str("i8042 timeout waiting for controller"),
            Self::KeyboardResetFailed => f.write_str("PS/2 keyboard reset failed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Timeout helper (x86_64 only)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
/// Maximum spin iterations before declaring a timeout.
const SPIN_TIMEOUT: u32 = 100_000;

// ---------------------------------------------------------------------------
// I8042 (x86_64 only)
// ---------------------------------------------------------------------------

/// An i8042 PS/2 controller handle.
///
/// Like [`Uart16550`](crate::uart16550::Uart16550), this type is `Copy` and
/// carries no state — the hardware retains its configuration from the last
/// [`init`](Self::init) call.
#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy)]
pub struct I8042;

#[cfg(target_arch = "x86_64")]
impl I8042 {
    /// Creates a new i8042 handle. Does **not** touch hardware.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns a port handle for the data port (0x60).
    #[inline]
    const fn data_port() -> Port<u8> {
        Port::new(DATA_PORT)
    }

    /// Returns a port handle for the status/command port (0x64).
    #[inline]
    const fn status_cmd_port() -> Port<u8> {
        Port::new(STATUS_CMD_PORT)
    }

    /// Reads the status register.
    #[must_use]
    pub fn read_status(&self) -> StatusReg {
        unsafe { StatusReg::from_bits_truncate(Self::status_cmd_port().read()) }
    }

    /// Reads one byte from the data port.
    ///
    /// # Safety
    ///
    /// Caller must verify the output buffer is full before reading.
    pub unsafe fn read_data(&self) -> u8 {
        unsafe { Self::data_port().read() }
    }

    /// Writes one byte to the data port.
    pub fn write_data(&self, byte: u8) {
        self.wait_input_ready();
        unsafe { Self::data_port().write(byte) };
    }

    /// Sends a command byte to the command port (0x64).
    pub fn send_command(&self, command: u8) {
        self.wait_input_ready();
        unsafe { Self::status_cmd_port().write(command) };
    }

    /// Spins until the input buffer is empty (controller ready for writes).
    fn wait_input_ready(&self) {
        for _ in 0..SPIN_TIMEOUT {
            if !self.read_status().contains(StatusReg::INPUT_FULL) {
                return;
            }
            core::hint::spin_loop();
        }
    }

    /// Spins until the output buffer is full (data available to read).
    /// Returns `true` if data became available, `false` on timeout.
    fn wait_output_ready(&self) -> bool {
        for _ in 0..SPIN_TIMEOUT {
            if self.read_status().contains(StatusReg::OUTPUT_FULL) {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    /// Flushes any pending data in the output buffer.
    fn flush_output(&self) {
        while self.read_status().contains(StatusReg::OUTPUT_FULL) {
            unsafe { Self::data_port().read() };
        }
    }

    /// Initializes the i8042 controller and the PS/2 keyboard.
    ///
    /// Follows the standard initialization sequence:
    /// 1. Disable both ports
    /// 2. Flush output buffer
    /// 3. Configure controller (disable IRQs during setup)
    /// 4. Self-test
    /// 5. Enable ports and IRQs
    /// 6. Reset keyboard device
    ///
    /// # Safety
    ///
    /// Must be called once before any other i8042 operations. The caller
    /// must ensure the I/O ports 0x60 and 0x64 are valid i8042 ports.
    pub unsafe fn init(&self) -> Result<(), InitError> {
        // 1. Disable both ports.
        self.send_command(cmd::DISABLE_PORT1);
        self.send_command(cmd::DISABLE_PORT2);

        // 2. Flush the output buffer.
        self.flush_output();

        // 3. Read config byte, clear IRQ enable bits, write back.
        self.send_command(cmd::READ_CONFIG);
        if !self.wait_output_ready() {
            return Err(InitError::Timeout);
        }
        let config = unsafe { Self::data_port().read() };
        let config = ConfigByte::from_bits_truncate(config);
        let config = config & !(ConfigByte::PORT1_IRQ | ConfigByte::PORT2_IRQ);
        self.send_command(cmd::WRITE_CONFIG);
        self.write_data(config.bits());

        // 4. Controller self-test.
        self.send_command(cmd::SELF_TEST);
        if !self.wait_output_ready() {
            return Err(InitError::Timeout);
        }
        let response = unsafe { Self::data_port().read() };
        if response != SELF_TEST_OK {
            return Err(InitError::SelfTestFailed);
        }

        // Restore config after self-test (some controllers reset it).
        self.send_command(cmd::WRITE_CONFIG);
        self.write_data(config.bits());

        // 5. Enable port 1 (keyboard) and port 2 (mouse).
        self.send_command(cmd::ENABLE_PORT1);
        self.send_command(cmd::ENABLE_PORT2);

        // 6. Re-enable IRQ bits in config byte.
        let config = config | ConfigByte::PORT1_IRQ | ConfigByte::PORT2_IRQ;
        self.send_command(cmd::WRITE_CONFIG);
        self.write_data(config.bits());

        // 7. Reset keyboard device (send 0xFF, expect 0xFA ack + 0xAA self-test).
        self.write_data(DEVICE_RESET);
        if !self.wait_output_ready() {
            return Err(InitError::KeyboardResetFailed);
        }
        let ack = unsafe { Self::data_port().read() };
        if ack != ACK {
            return Err(InitError::KeyboardResetFailed);
        }
        // Wait for self-test result.
        if !self.wait_output_ready() {
            return Err(InitError::KeyboardResetFailed);
        }
        let test = unsafe { Self::data_port().read() };
        if test != DEVICE_SELF_TEST_OK {
            return Err(InitError::KeyboardResetFailed);
        }

        Ok(())
    }

    /// Returns `true` if the output buffer has data from the keyboard
    /// (output full AND NOT mouse data bit).
    #[must_use]
    pub fn keyboard_data_available(&self) -> bool {
        let status = self.read_status();
        status.contains(StatusReg::OUTPUT_FULL) && !status.contains(StatusReg::MOUSE_DATA)
    }

    /// Returns `true` if the output buffer has data from the mouse
    /// (output full AND mouse data bit set).
    #[must_use]
    pub fn mouse_data_available(&self) -> bool {
        let status = self.read_status();
        status.contains(StatusReg::OUTPUT_FULL) && status.contains(StatusReg::MOUSE_DATA)
    }

    /// Non-blocking read: returns `Some(byte)` if keyboard data is available.
    #[must_use]
    pub fn try_read_keyboard(&self) -> Option<u8> {
        if self.keyboard_data_available() {
            Some(unsafe { Self::data_port().read() })
        } else {
            None
        }
    }

    /// Non-blocking read: returns `Some(byte)` if mouse data is available.
    #[must_use]
    pub fn try_read_mouse(&self) -> Option<u8> {
        if self.mouse_data_available() {
            Some(unsafe { Self::data_port().read() })
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Scancode translation (Set 1)
// ---------------------------------------------------------------------------

/// Translates a Set 1 scancode to a [`KeyCode`].
///
/// Returns `None` for unmapped scancodes.
#[must_use]
pub fn scancode_to_keycode(scancode: u8) -> Option<KeyCode> {
    // Strip the release bit (bit 7) to get the make code.
    let make = scancode & 0x7F;
    match make {
        0x01 => Some(KeyCode::Escape),
        0x02 => Some(KeyCode::Num1),
        0x03 => Some(KeyCode::Num2),
        0x04 => Some(KeyCode::Num3),
        0x05 => Some(KeyCode::Num4),
        0x06 => Some(KeyCode::Num5),
        0x07 => Some(KeyCode::Num6),
        0x08 => Some(KeyCode::Num7),
        0x09 => Some(KeyCode::Num8),
        0x0A => Some(KeyCode::Num9),
        0x0B => Some(KeyCode::Num0),
        0x0C => Some(KeyCode::Minus),
        0x0D => Some(KeyCode::Equals),
        0x0E => Some(KeyCode::Backspace),
        0x0F => Some(KeyCode::Tab),
        0x10 => Some(KeyCode::Q),
        0x11 => Some(KeyCode::W),
        0x12 => Some(KeyCode::E),
        0x13 => Some(KeyCode::R),
        0x14 => Some(KeyCode::T),
        0x15 => Some(KeyCode::Y),
        0x16 => Some(KeyCode::U),
        0x17 => Some(KeyCode::I),
        0x18 => Some(KeyCode::O),
        0x19 => Some(KeyCode::P),
        0x1A => Some(KeyCode::LeftBracket),
        0x1B => Some(KeyCode::RightBracket),
        0x1C => Some(KeyCode::Enter),
        0x1D => Some(KeyCode::LeftCtrl),
        0x1E => Some(KeyCode::A),
        0x1F => Some(KeyCode::S),
        0x20 => Some(KeyCode::D),
        0x21 => Some(KeyCode::F),
        0x22 => Some(KeyCode::G),
        0x23 => Some(KeyCode::H),
        0x24 => Some(KeyCode::J),
        0x25 => Some(KeyCode::K),
        0x26 => Some(KeyCode::L),
        0x27 => Some(KeyCode::Semicolon),
        0x28 => Some(KeyCode::Apostrophe),
        0x29 => Some(KeyCode::Grave),
        0x2A => Some(KeyCode::LeftShift),
        0x2B => Some(KeyCode::Backslash),
        0x2C => Some(KeyCode::Z),
        0x2D => Some(KeyCode::X),
        0x2E => Some(KeyCode::C),
        0x2F => Some(KeyCode::V),
        0x30 => Some(KeyCode::B),
        0x31 => Some(KeyCode::N),
        0x32 => Some(KeyCode::M),
        0x33 => Some(KeyCode::Comma),
        0x34 => Some(KeyCode::Period),
        0x35 => Some(KeyCode::Slash),
        0x36 => Some(KeyCode::RightShift),
        0x38 => Some(KeyCode::LeftAlt),
        0x39 => Some(KeyCode::Space),
        0x3A => Some(KeyCode::CapsLock),
        0x3B => Some(KeyCode::F1),
        0x3C => Some(KeyCode::F2),
        0x3D => Some(KeyCode::F3),
        0x3E => Some(KeyCode::F4),
        0x3F => Some(KeyCode::F5),
        0x40 => Some(KeyCode::F6),
        0x41 => Some(KeyCode::F7),
        0x42 => Some(KeyCode::F8),
        0x43 => Some(KeyCode::F9),
        0x44 => Some(KeyCode::F10),
        0x57 => Some(KeyCode::F11),
        0x58 => Some(KeyCode::F12),
        _ => None,
    }
}

/// Translates an extended (0xE0-prefixed) scancode to a [`KeyCode`].
#[must_use]
pub fn extended_scancode_to_keycode(scancode: u8) -> Option<KeyCode> {
    let make = scancode & 0x7F;
    match make {
        0x1D => Some(KeyCode::RightCtrl),
        0x38 => Some(KeyCode::RightAlt),
        0x47 => Some(KeyCode::Home),
        0x48 => Some(KeyCode::ArrowUp),
        0x49 => Some(KeyCode::PageUp),
        0x4B => Some(KeyCode::ArrowLeft),
        0x4D => Some(KeyCode::ArrowRight),
        0x4F => Some(KeyCode::End),
        0x50 => Some(KeyCode::ArrowDown),
        0x51 => Some(KeyCode::PageDown),
        0x52 => Some(KeyCode::Insert),
        0x53 => Some(KeyCode::Delete),
        _ => None,
    }
}

/// Returns `true` if the scancode represents a key release (bit 7 set).
#[must_use]
pub const fn is_release(scancode: u8) -> bool {
    scancode & 0x80 != 0
}

// ---------------------------------------------------------------------------
// Mouse packet parsing
// ---------------------------------------------------------------------------

/// A parsed 3-byte PS/2 mouse packet.
#[derive(Debug, Clone, Copy)]
pub struct MousePacket {
    /// Relative X movement.
    pub dx: i16,
    /// Relative Y movement.
    pub dy: i16,
    /// Left button pressed.
    pub left: bool,
    /// Right button pressed.
    pub right: bool,
    /// Middle button pressed.
    pub middle: bool,
}

impl MousePacket {
    /// Parses a 3-byte PS/2 mouse packet.
    ///
    /// `bytes[0]` = status byte (buttons + sign bits + overflow)
    /// `bytes[1]` = X movement
    /// `bytes[2]` = Y movement
    #[must_use]
    pub fn parse(bytes: [u8; 3]) -> Self {
        let status = bytes[0];
        let left = status & 0x01 != 0;
        let right = status & 0x02 != 0;
        let middle = status & 0x04 != 0;
        let x_sign = status & 0x10 != 0;
        let y_sign = status & 0x20 != 0;

        // Sign-extend the movement values.
        let dx = if x_sign {
            i16::from(bytes[1]) - 256
        } else {
            i16::from(bytes[1])
        };
        let dy = if y_sign {
            i16::from(bytes[2]) - 256
        } else {
            i16::from(bytes[2])
        };

        Self {
            dx,
            dy,
            left,
            right,
            middle,
        }
    }
}

// ---------------------------------------------------------------------------
// Driver registration
// ---------------------------------------------------------------------------

// Platform driver entry for the i8042 PS/2 controller.
// Matched by compatible string "i8042". The actual async setup happens
// in hadron-kernel's AsyncKeyboard/AsyncMouse wrappers.
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
hadron_driver_api::platform_driver_entry!(
    I8042_DRIVER,
    hadron_driver_api::registration::PlatformDriverEntry {
        name: "i8042",
        compatible: "i8042",
        init: i8042_platform_init,
    }
);

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
fn i8042_platform_init(
    _services: &'static dyn hadron_driver_api::services::KernelServices,
) -> Result<(), hadron_driver_api::error::DriverError> {
    // Platform init hook — performs the actual controller initialization.
    let ctrl = I8042::new();
    // SAFETY: Called once during driver matching, ports 0x60/0x64 are valid i8042.
    unsafe { ctrl.init() }.map_err(|_| hadron_driver_api::error::DriverError::InitFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- scancode_to_keycode tests --

    #[test]
    fn scancode_escape() {
        assert_eq!(scancode_to_keycode(0x01), Some(KeyCode::Escape));
    }

    #[test]
    fn scancode_letters() {
        assert_eq!(scancode_to_keycode(0x10), Some(KeyCode::Q));
        assert_eq!(scancode_to_keycode(0x1E), Some(KeyCode::A));
        assert_eq!(scancode_to_keycode(0x2C), Some(KeyCode::Z));
    }

    #[test]
    fn scancode_release_strips_bit7() {
        // Release scancode for 'A' (0x1E | 0x80 = 0x9E).
        assert_eq!(scancode_to_keycode(0x9E), Some(KeyCode::A));
    }

    #[test]
    fn scancode_unknown() {
        assert_eq!(scancode_to_keycode(0x7F), None);
    }

    #[test]
    fn is_release_flag() {
        assert!(!is_release(0x1E)); // A press
        assert!(is_release(0x9E)); // A release
        assert!(is_release(0x80));
        assert!(!is_release(0x00));
    }

    #[test]
    fn extended_scancode_arrows() {
        assert_eq!(extended_scancode_to_keycode(0x48), Some(KeyCode::ArrowUp));
        assert_eq!(extended_scancode_to_keycode(0x50), Some(KeyCode::ArrowDown));
        assert_eq!(extended_scancode_to_keycode(0x4B), Some(KeyCode::ArrowLeft));
        assert_eq!(
            extended_scancode_to_keycode(0x4D),
            Some(KeyCode::ArrowRight)
        );
    }

    #[test]
    fn extended_scancode_unknown() {
        assert_eq!(extended_scancode_to_keycode(0x00), None);
    }

    // -- MousePacket::parse tests --

    #[test]
    fn mouse_no_movement_no_buttons() {
        let pkt = MousePacket::parse([0x00, 0, 0]);
        assert_eq!(pkt.dx, 0);
        assert_eq!(pkt.dy, 0);
        assert!(!pkt.left);
        assert!(!pkt.right);
        assert!(!pkt.middle);
    }

    #[test]
    fn mouse_positive_movement() {
        let pkt = MousePacket::parse([0x00, 10, 20]);
        assert_eq!(pkt.dx, 10);
        assert_eq!(pkt.dy, 20);
    }

    #[test]
    fn mouse_negative_movement() {
        // X sign bit set (bit 4 of status), Y sign bit set (bit 5).
        let pkt = MousePacket::parse([0x30, 246, 236]);
        assert_eq!(pkt.dx, 246 - 256); // -10
        assert_eq!(pkt.dy, 236 - 256); // -20
    }

    #[test]
    fn mouse_buttons() {
        let pkt = MousePacket::parse([0x01, 0, 0]);
        assert!(pkt.left);
        assert!(!pkt.right);
        assert!(!pkt.middle);

        let pkt = MousePacket::parse([0x02, 0, 0]);
        assert!(!pkt.left);
        assert!(pkt.right);

        let pkt = MousePacket::parse([0x04, 0, 0]);
        assert!(pkt.middle);

        let pkt = MousePacket::parse([0x07, 0, 0]);
        assert!(pkt.left);
        assert!(pkt.right);
        assert!(pkt.middle);
    }
}
