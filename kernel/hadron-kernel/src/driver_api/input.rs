//! Input device interface traits (keyboard and mouse).

use super::error::DriverError;

/// Key codes for common keyboard keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    // Letters
    /// A key.
    A,
    /// B key.
    B,
    /// C key.
    C,
    /// D key.
    D,
    /// E key.
    E,
    /// F key.
    F,
    /// G key.
    G,
    /// H key.
    H,
    /// I key.
    I,
    /// J key.
    J,
    /// K key.
    K,
    /// L key.
    L,
    /// M key.
    M,
    /// N key.
    N,
    /// O key.
    O,
    /// P key.
    P,
    /// Q key.
    Q,
    /// R key.
    R,
    /// S key.
    S,
    /// T key.
    T,
    /// U key.
    U,
    /// V key.
    V,
    /// W key.
    W,
    /// X key.
    X,
    /// Y key.
    Y,
    /// Z key.
    Z,

    // Digits
    /// 0 key.
    Num0,
    /// 1 key.
    Num1,
    /// 2 key.
    Num2,
    /// 3 key.
    Num3,
    /// 4 key.
    Num4,
    /// 5 key.
    Num5,
    /// 6 key.
    Num6,
    /// 7 key.
    Num7,
    /// 8 key.
    Num8,
    /// 9 key.
    Num9,

    // Function keys
    /// F1 key.
    F1,
    /// F2 key.
    F2,
    /// F3 key.
    F3,
    /// F4 key.
    F4,
    /// F5 key.
    F5,
    /// F6 key.
    F6,
    /// F7 key.
    F7,
    /// F8 key.
    F8,
    /// F9 key.
    F9,
    /// F10 key.
    F10,
    /// F11 key.
    F11,
    /// F12 key.
    F12,

    // Modifiers
    /// Left Shift key.
    LeftShift,
    /// Right Shift key.
    RightShift,
    /// Left Control key.
    LeftCtrl,
    /// Right Control key.
    RightCtrl,
    /// Left Alt key.
    LeftAlt,
    /// Right Alt key.
    RightAlt,
    /// Caps Lock key.
    CapsLock,

    // Navigation
    /// Up arrow key.
    ArrowUp,
    /// Down arrow key.
    ArrowDown,
    /// Left arrow key.
    ArrowLeft,
    /// Right arrow key.
    ArrowRight,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page Up key.
    PageUp,
    /// Page Down key.
    PageDown,
    /// Insert key.
    Insert,
    /// Delete key.
    Delete,

    // Common keys
    /// Escape key.
    Escape,
    /// Enter/Return key.
    Enter,
    /// Tab key.
    Tab,
    /// Backspace key.
    Backspace,
    /// Space bar.
    Space,

    // Punctuation and symbols
    /// Minus/Underscore key.
    Minus,
    /// Equals/Plus key.
    Equals,
    /// Left bracket key.
    LeftBracket,
    /// Right bracket key.
    RightBracket,
    /// Backslash/Pipe key.
    Backslash,
    /// Semicolon/Colon key.
    Semicolon,
    /// Apostrophe/Quote key.
    Apostrophe,
    /// Grave/Tilde key.
    Grave,
    /// Comma/Less-than key.
    Comma,
    /// Period/Greater-than key.
    Period,
    /// Slash/Question key.
    Slash,

    /// Unknown or unmapped scancode.
    Unknown(u8),
}

/// A keyboard event (press or release).
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    /// The key that was pressed or released.
    pub key: KeyCode,
    /// `true` if pressed, `false` if released.
    pub pressed: bool,
}

/// A mouse event with relative movement and button state.
#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
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

/// Interface trait for keyboard input devices.
///
/// Provides key event reading via interrupt-driven async I/O.
#[expect(async_fn_in_trait, reason = "internal trait, no dyn dispatch needed")]
pub trait KeyboardDevice {
    /// Reads the next key event, waiting if necessary.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError`] if the read fails.
    async fn read_event(&self) -> Result<KeyEvent, DriverError>;

    /// Returns `true` if a key event is available to read without blocking.
    fn event_available(&self) -> bool;
}

/// Interface trait for mouse input devices.
///
/// Provides mouse event reading via interrupt-driven async I/O.
#[expect(async_fn_in_trait, reason = "internal trait, no dyn dispatch needed")]
pub trait MouseDevice {
    /// Reads the next mouse event, waiting if necessary.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError`] if the read fails.
    async fn read_event(&self) -> Result<MouseEvent, DriverError>;

    /// Returns `true` if a mouse event is available to read without blocking.
    fn event_available(&self) -> bool;
}
