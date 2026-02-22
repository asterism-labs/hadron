//! Line discipline — cooked-mode line editing and scancode processing.
//!
//! Handles buffering, backspace, Enter, Ctrl+C (interrupt), Ctrl+D (EOF),
//! shift, caps lock, and extended scancode decoding. Produces [`LdiscAction`]
//! events that the owning [`super::Tty`] interprets (echo, signal delivery,
//! etc.).

use crate::driver_api::input::KeyCode;
use planck_noalloc::ringbuf::RingBuf;

/// Maximum line length for cooked-mode editing.
const LINE_BUF_SIZE: usize = 256;

/// Size of the ring buffer backing store (usable capacity is SIZE - 1).
const READY_BUF_SIZE: usize = 512;

/// Action returned by [`LineDiscipline::process_scancode`].
///
/// The caller (Tty) uses these to drive echo output and signal delivery.
pub enum LdiscAction {
    /// Erase one character: cursor back, space, cursor back.
    Backspace,
    /// Print a newline (Enter pressed, line committed to ready buffer).
    Newline,
    /// Print a single ASCII character.
    Char(u8),
    /// Ctrl+C: line discarded, caller should send SIGINT to foreground.
    Interrupt,
    /// Ctrl+D on empty line: EOF marker set.
    Eof,
    /// Ctrl+D on non-empty line: line flushed without trailing newline.
    FlushLine,
    /// Alt+Fn: request VT switch to the given index (0-based).
    SwitchVt(usize),
}

/// Cooked-mode line discipline state.
///
/// Buffers characters in a line buffer until Enter (or Ctrl+D flush) commits
/// them to the ready buffer for userspace reads.
pub struct LineDiscipline {
    /// Completed lines ready for userspace reads.
    ready_buf: RingBuf<u8, READY_BUF_SIZE>,
    /// Current line being edited.
    line_buf: [u8; LINE_BUF_SIZE],
    /// Number of bytes in the current line buffer.
    line_len: usize,
    /// Whether a shift key is currently held.
    shift_held: bool,
    /// Whether a ctrl key is currently held.
    ctrl_held: bool,
    /// Whether an alt key is currently held.
    alt_held: bool,
    /// Whether caps lock is active (toggled).
    caps_lock: bool,
    /// Whether the previous scancode was the 0xE0 extended prefix.
    extended_prefix: bool,
    /// Whether an EOF condition (Ctrl+D on empty line) is pending.
    eof_pending: bool,
}

impl LineDiscipline {
    /// Create a new line discipline in cooked mode.
    pub const fn new() -> Self {
        Self {
            ready_buf: RingBuf::new(),
            line_buf: [0; LINE_BUF_SIZE],
            line_len: 0,
            shift_held: false,
            ctrl_held: false,
            alt_held: false,
            caps_lock: false,
            extended_prefix: false,
            eof_pending: false,
        }
    }

    /// Process a single raw scancode.
    ///
    /// Decodes the scancode, updates modifier state, and buffers the resulting
    /// character in cooked mode. Returns a [`LdiscAction`] describing what the
    /// caller should do (echo, signal, etc.).
    pub fn process_scancode(&mut self, scancode: u8) -> Option<LdiscAction> {
        // Handle 0xE0 extended prefix.
        if scancode == 0xE0 {
            self.extended_prefix = true;
            return None;
        }

        let is_release = scancode & 0x80 != 0;

        let keycode = if self.extended_prefix {
            self.extended_prefix = false;
            extended_scancode_to_keycode(scancode)
        } else {
            scancode_to_keycode(scancode)
        };

        let Some(key) = keycode else {
            return None;
        };

        // Update modifier state.
        match key {
            KeyCode::LeftShift | KeyCode::RightShift => {
                self.shift_held = !is_release;
                return None;
            }
            KeyCode::LeftCtrl | KeyCode::RightCtrl => {
                self.ctrl_held = !is_release;
                return None;
            }
            KeyCode::LeftAlt | KeyCode::RightAlt => {
                self.alt_held = !is_release;
                return None;
            }
            KeyCode::CapsLock => {
                if !is_release {
                    self.caps_lock = !self.caps_lock;
                }
                return None;
            }
            _ => {}
        }

        // Only process key presses, not releases.
        if is_release {
            return None;
        }

        // Alt+F1..F6: VT switching.
        if self.alt_held {
            match key {
                KeyCode::F1 => return Some(LdiscAction::SwitchVt(0)),
                KeyCode::F2 => return Some(LdiscAction::SwitchVt(1)),
                KeyCode::F3 => return Some(LdiscAction::SwitchVt(2)),
                KeyCode::F4 => return Some(LdiscAction::SwitchVt(3)),
                KeyCode::F5 => return Some(LdiscAction::SwitchVt(4)),
                KeyCode::F6 => return Some(LdiscAction::SwitchVt(5)),
                _ => {}
            }
        }

        // Ctrl+C: interrupt.
        if self.ctrl_held && key == KeyCode::C {
            self.line_len = 0;
            return Some(LdiscAction::Interrupt);
        }

        // Ctrl+D: EOF on empty line, flush on non-empty line.
        if self.ctrl_held && key == KeyCode::D {
            if self.line_len == 0 {
                self.eof_pending = true;
                return Some(LdiscAction::Eof);
            }
            // Flush current line without trailing newline.
            let len = self.line_len;
            for i in 0..len {
                let byte = self.line_buf[i];
                let _ = self.ready_buf.try_push(byte);
            }
            self.line_len = 0;
            return Some(LdiscAction::FlushLine);
        }

        let shifted = self.shift_held;
        let caps = self.caps_lock;

        if let Some(ch) = keycode_to_ascii(key, shifted, caps) {
            match ch {
                b'\x08' => {
                    // Backspace: erase one character.
                    if self.line_len > 0 {
                        self.line_len -= 1;
                        return Some(LdiscAction::Backspace);
                    }
                }
                b'\n' => {
                    // Enter: copy line + newline into ready buffer.
                    let len = self.line_len;
                    for i in 0..len {
                        let byte = self.line_buf[i];
                        let _ = self.ready_buf.try_push(byte);
                    }
                    let _ = self.ready_buf.try_push(b'\n');
                    self.line_len = 0;
                    return Some(LdiscAction::Newline);
                }
                _ => {
                    // Printable character: append to line buffer if there's room.
                    let len = self.line_len;
                    if len < LINE_BUF_SIZE {
                        self.line_buf[len] = ch;
                        self.line_len += 1;
                        return Some(LdiscAction::Char(ch));
                    }
                }
            }
        }
        None
    }

    /// Non-blocking read from the completed-line buffer.
    ///
    /// Returns `Some(n)` with the number of bytes copied into `buf`, `Some(0)`
    /// if EOF was signaled (Ctrl+D on empty line), or `None` if no data is
    /// available yet.
    pub fn try_read(&mut self, buf: &mut [u8]) -> Option<usize> {
        // Check for pending EOF.
        if self.eof_pending {
            self.eof_pending = false;
            return Some(0);
        }

        let mut n = 0;
        for slot in buf.iter_mut() {
            match self.ready_buf.pop() {
                Some(byte) => {
                    *slot = byte;
                    n += 1;
                }
                None => break,
            }
        }

        if n > 0 { Some(n) } else { None }
    }
}

// ── Scancode decoding ──────────────────────────────────────────────

/// Translates a Set 1 scancode to a [`KeyCode`].
fn scancode_to_keycode(scancode: u8) -> Option<KeyCode> {
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
fn extended_scancode_to_keycode(scancode: u8) -> Option<KeyCode> {
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

/// Translate a [`KeyCode`] to an ASCII byte, accounting for shift and caps lock.
///
/// Returns `None` for keys that don't produce a character (function keys,
/// arrows, modifiers, etc.).
fn keycode_to_ascii(key: KeyCode, shifted: bool, caps: bool) -> Option<u8> {
    match key {
        // Letters: caps lock XOR shift determines case.
        KeyCode::A => Some(if shifted ^ caps { b'A' } else { b'a' }),
        KeyCode::B => Some(if shifted ^ caps { b'B' } else { b'b' }),
        KeyCode::C => Some(if shifted ^ caps { b'C' } else { b'c' }),
        KeyCode::D => Some(if shifted ^ caps { b'D' } else { b'd' }),
        KeyCode::E => Some(if shifted ^ caps { b'E' } else { b'e' }),
        KeyCode::F => Some(if shifted ^ caps { b'F' } else { b'f' }),
        KeyCode::G => Some(if shifted ^ caps { b'G' } else { b'g' }),
        KeyCode::H => Some(if shifted ^ caps { b'H' } else { b'h' }),
        KeyCode::I => Some(if shifted ^ caps { b'I' } else { b'i' }),
        KeyCode::J => Some(if shifted ^ caps { b'J' } else { b'j' }),
        KeyCode::K => Some(if shifted ^ caps { b'K' } else { b'k' }),
        KeyCode::L => Some(if shifted ^ caps { b'L' } else { b'l' }),
        KeyCode::M => Some(if shifted ^ caps { b'M' } else { b'm' }),
        KeyCode::N => Some(if shifted ^ caps { b'N' } else { b'n' }),
        KeyCode::O => Some(if shifted ^ caps { b'O' } else { b'o' }),
        KeyCode::P => Some(if shifted ^ caps { b'P' } else { b'p' }),
        KeyCode::Q => Some(if shifted ^ caps { b'Q' } else { b'q' }),
        KeyCode::R => Some(if shifted ^ caps { b'R' } else { b'r' }),
        KeyCode::S => Some(if shifted ^ caps { b'S' } else { b's' }),
        KeyCode::T => Some(if shifted ^ caps { b'T' } else { b't' }),
        KeyCode::U => Some(if shifted ^ caps { b'U' } else { b'u' }),
        KeyCode::V => Some(if shifted ^ caps { b'V' } else { b'v' }),
        KeyCode::W => Some(if shifted ^ caps { b'W' } else { b'w' }),
        KeyCode::X => Some(if shifted ^ caps { b'X' } else { b'x' }),
        KeyCode::Y => Some(if shifted ^ caps { b'Y' } else { b'y' }),
        KeyCode::Z => Some(if shifted ^ caps { b'Z' } else { b'z' }),

        // Digits (shift produces symbols).
        KeyCode::Num1 => Some(if shifted { b'!' } else { b'1' }),
        KeyCode::Num2 => Some(if shifted { b'@' } else { b'2' }),
        KeyCode::Num3 => Some(if shifted { b'#' } else { b'3' }),
        KeyCode::Num4 => Some(if shifted { b'$' } else { b'4' }),
        KeyCode::Num5 => Some(if shifted { b'%' } else { b'5' }),
        KeyCode::Num6 => Some(if shifted { b'^' } else { b'6' }),
        KeyCode::Num7 => Some(if shifted { b'&' } else { b'7' }),
        KeyCode::Num8 => Some(if shifted { b'*' } else { b'8' }),
        KeyCode::Num9 => Some(if shifted { b'(' } else { b'9' }),
        KeyCode::Num0 => Some(if shifted { b')' } else { b'0' }),

        // Punctuation.
        KeyCode::Minus => Some(if shifted { b'_' } else { b'-' }),
        KeyCode::Equals => Some(if shifted { b'+' } else { b'=' }),
        KeyCode::LeftBracket => Some(if shifted { b'{' } else { b'[' }),
        KeyCode::RightBracket => Some(if shifted { b'}' } else { b']' }),
        KeyCode::Backslash => Some(if shifted { b'|' } else { b'\\' }),
        KeyCode::Semicolon => Some(if shifted { b':' } else { b';' }),
        KeyCode::Apostrophe => Some(if shifted { b'"' } else { b'\'' }),
        KeyCode::Grave => Some(if shifted { b'~' } else { b'`' }),
        KeyCode::Comma => Some(if shifted { b'<' } else { b',' }),
        KeyCode::Period => Some(if shifted { b'>' } else { b'.' }),
        KeyCode::Slash => Some(if shifted { b'?' } else { b'/' }),

        // Special keys.
        KeyCode::Space => Some(b' '),
        KeyCode::Enter => Some(b'\n'),
        KeyCode::Backspace => Some(b'\x08'),
        KeyCode::Tab => Some(b'\t'),

        // Non-character keys.
        _ => None,
    }
}
