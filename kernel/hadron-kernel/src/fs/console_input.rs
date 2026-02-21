//! Console input subsystem for `/dev/console` reads.
//!
//! Provides cooked-mode line editing: characters are buffered in a line buffer
//! until Enter is pressed, then the completed line (with trailing newline) is
//! copied into a ready buffer that userspace can read from.
//!
//! Keyboard input is driven by IRQ1: the interrupt handler drains the PS/2
//! hardware FIFO into a software ring buffer, then wakes futures waiting on
//! the [`INPUT_READY`] wait queue. Deferred scancode processing (echo, line
//! editing) happens in [`poll_keyboard_hardware`] when the reader future is
//! polled by the executor.

use crate::sync::IrqSpinLock;
use crate::driver_api::input::KeyCode;
use noalloc::ringbuf::RingBuf;

use crate::sync::HeapWaitQueue;

/// Woken by the keyboard IRQ handler when a scancode is available.
static INPUT_READY: HeapWaitQueue = HeapWaitQueue::new();

/// Maximum line length for cooked-mode editing.
const LINE_BUF_SIZE: usize = 256;

/// Size of the ring buffer backing store (usable capacity is SIZE - 1).
const READY_BUF_SIZE: usize = 512;

/// Capacity of the IRQ-filled scancode ring buffer.
const SCANCODE_BUF_SIZE: usize = 64;

/// Scancode ring buffer filled by the keyboard IRQ handler.
///
/// The IRQ handler drains the PS/2 hardware FIFO into this buffer so that
/// scancodes are preserved even if a waker notification is lost (e.g. consumed
/// by a noop waker from [`crate::fs::try_poll_immediate`]). Processing and
/// echo happen later in [`poll_keyboard_hardware`].
static SCANCODE_BUF: IrqSpinLock<RingBuf<u8, SCANCODE_BUF_SIZE>> =
    IrqSpinLock::new(RingBuf::new());

/// Global console input state, protected by an IRQ-safe spinlock.
///
/// Uses [`IrqSpinLock`] because [`poll_keyboard_hardware`] accesses this
/// while the keyboard IRQ handler could fire on the same CPU.
static STATE: IrqSpinLock<ConsoleInputState> = IrqSpinLock::new(ConsoleInputState::new());

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

/// Reads a keyboard scancode from the PS/2 data port, if available.
///
/// Checks the i8042 status register (port 0x64) for output-buffer-full
/// without the mouse-data bit, then reads the data port (0x60).
#[cfg(target_arch = "x86_64")]
fn try_read_keyboard_scancode() -> Option<u8> {
    use crate::arch::x86_64::Port;
    // SAFETY: Reading status and data ports is a standard PS/2 operation.
    let status = unsafe { Port::<u8>::new(0x64).read() };
    // Bit 0: output buffer full, bit 5: mouse data.
    if status & 0x01 != 0 && status & 0x20 == 0 {
        Some(unsafe { Port::<u8>::new(0x60).read() })
    } else {
        None
    }
}

/// Returns `true` if the scancode represents a key release (bit 7 set).
const fn is_release(scancode: u8) -> bool {
    scancode & 0x80 != 0
}

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

/// Process a single raw scancode: decode, update modifier state, echo, and
/// buffer the resulting character in cooked mode.
fn process_scancode(state: &mut ConsoleInputState, scancode: u8) {
    // Handle 0xE0 extended prefix.
    if scancode == 0xE0 {
        state.extended_prefix = true;
        return;
    }

    let is_release = is_release(scancode);

    let keycode = if state.extended_prefix {
        state.extended_prefix = false;
        extended_scancode_to_keycode(scancode)
    } else {
        scancode_to_keycode(scancode)
    };

    let Some(key) = keycode else {
        return;
    };

    // Update modifier state.
    match key {
        KeyCode::LeftShift | KeyCode::RightShift => {
            state.shift_held = !is_release;
            return;
        }
        KeyCode::CapsLock => {
            if !is_release {
                state.caps_lock = !state.caps_lock;
            }
            return;
        }
        _ => {}
    }

    // Only process key presses, not releases.
    if is_release {
        return;
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
                    crate::kprint!("\x08 \x08");
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
                crate::kprint!("\n");
            }
            _ => {
                // Printable character: append to line buffer if there's room.
                let len = state.line_len;
                if len < LINE_BUF_SIZE {
                    state.line_buf[len] = ch;
                    state.line_len += 1;
                    crate::kprint!("{}", ch as char);
                }
            }
        }
    }
}

/// Drain buffered scancodes, decode them, and push ASCII into the line buffer.
///
/// Processes scancodes from two sources:
/// 1. The [`SCANCODE_BUF`] ring buffer (filled by the IRQ handler)
/// 2. The PS/2 hardware FIFO directly (catches bytes from before IRQ setup)
///
/// Scancodes are collected with interrupts disabled to prevent races with
/// the IRQ handler, then processed under the [`STATE`] lock (which echoes
/// characters via `kprint!`).
pub fn poll_keyboard_hardware() {
    // Collect raw scancodes with interrupts disabled (no logger lock needed).
    let mut raw = [0u8; SCANCODE_BUF_SIZE];
    let mut count = 0;

    {
        let mut buf = SCANCODE_BUF.lock();

        // Drain the IRQ-buffered scancodes.
        while let Some(sc) = buf.pop() {
            if count < raw.len() {
                raw[count] = sc;
                count += 1;
            }
        }

        // Also drain hardware FIFO directly (catches bytes from before IRQ
        // setup, or any that arrived between IRQ handler and this call).
        // Interrupts are disabled (IrqSpinLock), so no race with the handler.
        while let Some(sc) = try_read_keyboard_scancode() {
            if count < raw.len() {
                raw[count] = sc;
                count += 1;
            }
        }
    }

    if count == 0 {
        return;
    }

    // Process all collected scancodes (echo + line editing).
    let mut state = STATE.lock();
    for &scancode in &raw[..count] {
        process_scancode(&mut state, scancode);
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

/// Keyboard IRQ handler — drains the PS/2 FIFO and wakes waiting readers.
///
/// Scancodes are buffered in [`SCANCODE_BUF`] for deferred processing by
/// [`poll_keyboard_hardware`]. Echo and line editing happen there (in thread
/// context) to avoid taking the logger lock from interrupt context.
fn keyboard_irq_handler(_vector: u8) {
    {
        let mut buf = SCANCODE_BUF.lock();
        while let Some(scancode) = try_read_keyboard_scancode() {
            let _ = buf.try_push(scancode);
        }
    }
    INPUT_READY.wake_all();
}

/// Initialize IRQ-driven keyboard input.
///
/// Registers the keyboard IRQ1 handler and unmasks it in the I/O APIC.
/// Must be called after APIC initialization.
pub fn init() {
    use crate::arch::x86_64::interrupts::dispatch;

    let vector = dispatch::vectors::isa_irq_vector(1);
    dispatch::register_handler(vector, keyboard_irq_handler)
        .expect("console_input: failed to register keyboard IRQ handler");

    crate::arch::x86_64::acpi::with_io_apic(|ioapic| ioapic.unmask(1));

    crate::kinfo!("Console input: keyboard IRQ1 enabled (vector {})", vector);
}

/// Registers a waker to be notified when keyboard input arrives.
pub fn subscribe(waker: &core::task::Waker) {
    INPUT_READY.register_waker(waker);
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
