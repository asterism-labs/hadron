//! Console input subsystem for `/dev/console` reads.
//!
//! Provides cooked-mode line editing: characters are buffered in a line buffer
//! until Enter is pressed, then the completed line (with trailing newline) is
//! copied into a ready buffer that userspace can read from.
//!
//! Keyboard hardware is polled synchronously via the i8042 PS/2 controller.

use hadron_core::sync::SpinLock;
use hadron_driver_api::input::KeyCode;
use hadron_drivers::i8042::{self, I8042};
use noalloc::ringbuf::RingBuf;

/// Maximum line length for cooked-mode editing.
const LINE_BUF_SIZE: usize = 256;

/// Size of the ring buffer backing store (usable capacity is SIZE - 1).
const READY_BUF_SIZE: usize = 512;

/// Global console input state, protected by a spinlock.
static STATE: SpinLock<ConsoleInputState> = SpinLock::new(ConsoleInputState::new());

/// Internal state for the console input subsystem.
struct ConsoleInputState {
    /// Completed lines ready for userspace reads.
    ready_buf: RingBuf<u8, READY_BUF_SIZE>,
    /// Current line being edited (cooked mode).
    line_buf: [u8; LINE_BUF_SIZE],
    /// Number of bytes in the current line buffer.
    line_len: usize,
    /// Whether a shift key is currently held.
    shift_held: bool,
    /// Whether caps lock is active (toggled).
    caps_lock: bool,
    /// Whether the previous scancode was the 0xE0 extended prefix.
    extended_prefix: bool,
}

impl ConsoleInputState {
    const fn new() -> Self {
        Self {
            ready_buf: RingBuf::new(),
            line_buf: [0; LINE_BUF_SIZE],
            line_len: 0,
            shift_held: false,
            caps_lock: false,
            extended_prefix: false,
        }
    }
}

/// Poll i8042 hardware directly, decode scancodes, and push ASCII into the buffer.
///
/// Reads all available scancodes from the PS/2 controller, translates them
/// to ASCII using the current modifier state, and performs cooked-mode line
/// editing (echo, backspace, enter).
pub fn poll_keyboard_hardware() {
    let controller = I8042::new();
    let mut state = STATE.lock();

    // Drain all available scancodes from the hardware.
    while let Some(scancode) = controller.try_read_keyboard() {
        // Handle 0xE0 extended prefix.
        if scancode == 0xE0 {
            state.extended_prefix = true;
            continue;
        }

        let is_release = i8042::is_release(scancode);

        let keycode = if state.extended_prefix {
            state.extended_prefix = false;
            i8042::extended_scancode_to_keycode(scancode)
        } else {
            i8042::scancode_to_keycode(scancode)
        };

        let Some(key) = keycode else {
            continue;
        };

        // Update modifier state.
        match key {
            KeyCode::LeftShift | KeyCode::RightShift => {
                state.shift_held = !is_release;
                continue;
            }
            KeyCode::CapsLock => {
                if !is_release {
                    state.caps_lock = !state.caps_lock;
                }
                continue;
            }
            _ => {}
        }

        // Only process key presses, not releases.
        if is_release {
            continue;
        }

        let shifted = state.shift_held;
        let caps = state.caps_lock;

        if let Some(ch) = keycode_to_ascii(key, shifted, caps) {
            match ch {
                b'\x08' => {
                    // Backspace: erase one character.
                    if state.line_len > 0 {
                        state.line_len -= 1;
                        // Echo: move cursor back, overwrite with space, move back again.
                        hadron_core::kprint!("\x08 \x08");
                    }
                }
                b'\n' => {
                    // Enter: copy line + newline into ready buffer.
                    let len = state.line_len;
                    for i in 0..len {
                        let byte = state.line_buf[i];
                        let _ = state.ready_buf.try_push(byte);
                    }
                    let _ = state.ready_buf.try_push(b'\n');
                    state.line_len = 0;
                    hadron_core::kprint!("\n");
                }
                _ => {
                    // Printable character: append to line buffer if there's room.
                    let len = state.line_len;
                    if len < LINE_BUF_SIZE {
                        state.line_buf[len] = ch;
                        state.line_len += 1;
                        hadron_core::kprint!("{}", ch as char);
                    }
                }
            }
        }
    }
}

/// Non-blocking read from the completed-line buffer.
///
/// Copies up to `buf.len()` bytes from the ready buffer into `buf`.
/// Returns the number of bytes actually copied.
pub fn try_read(buf: &mut [u8]) -> usize {
    let mut state = STATE.lock();
    let mut n = 0;
    for slot in buf.iter_mut() {
        match state.ready_buf.pop() {
            Some(byte) => {
                *slot = byte;
                n += 1;
            }
            None => break,
        }
    }
    n
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
