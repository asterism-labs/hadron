//! ANSI/VT escape sequence parser.
//!
//! Pure `no_std` state machine that processes one byte at a time. Returns
//! [`Action`] values that the caller translates into cell/cursor operations.
//!
//! Ported from `hadron-kernel`'s `fbcon::ansi` module.

/// Maximum number of CSI numeric parameters we track.
const MAX_PARAMS: usize = 16;

/// Parser state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Normal text mode.
    Ground,
    /// Received ESC (0x1B), waiting for next byte.
    Escape,
    /// Received ESC [ — entering CSI sequence.
    CsiParam,
    /// CSI intermediate bytes (0x20..=0x2F).
    CsiIntermediate,
    /// OSC string (ESC ]) — ignored, consumed until ST.
    OscString,
}

/// Action returned by the parser after processing one byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// No visible action (absorbed by parser state machine).
    None,
    /// Print a visible character at the current cursor position.
    Print(u8),
    /// Execute a C0 control character (e.g. `\n`, `\r`, `\t`, `\x08`).
    Execute(u8),
    /// Dispatch a CSI (Control Sequence Introducer) command.
    CsiDispatch {
        /// Numeric parameters (0 = default/absent).
        params: [u16; MAX_PARAMS],
        /// Number of valid parameters.
        param_count: usize,
        /// The final byte that identifies the command (e.g. `m`, `H`, `J`).
        final_byte: u8,
    },
}

/// ANSI escape sequence parser.
///
/// Feed bytes one at a time via [`feed`](Self::feed). The parser returns an
/// [`Action`] for each byte: either `None` (byte consumed internally),
/// `Print` (display a glyph), `Execute` (handle a control char), or
/// `CsiDispatch` (handle a CSI escape sequence).
pub struct AnsiParser {
    state: State,
    params: [u16; MAX_PARAMS],
    param_count: usize,
    /// Whether the current parameter has received any digit.
    param_started: bool,
}

impl AnsiParser {
    /// Creates a new parser in the ground state.
    pub const fn new() -> Self {
        Self {
            state: State::Ground,
            params: [0u16; MAX_PARAMS],
            param_count: 0,
            param_started: false,
        }
    }

    /// Resets parser state for CSI entry.
    fn csi_reset(&mut self) {
        self.params = [0u16; MAX_PARAMS];
        self.param_count = 0;
        self.param_started = false;
    }

    /// Feeds a single byte to the parser and returns the resulting action.
    pub fn feed(&mut self, byte: u8) -> Action {
        match self.state {
            State::Ground => self.ground(byte),
            State::Escape => self.escape(byte),
            State::CsiParam => self.csi_param(byte),
            State::CsiIntermediate => self.csi_intermediate(byte),
            State::OscString => self.osc_string(byte),
        }
    }

    fn ground(&mut self, byte: u8) -> Action {
        match byte {
            0x1B => {
                self.state = State::Escape;
                Action::None
            }
            // C0 control characters
            0x00..=0x1A | 0x1C..=0x1F => Action::Execute(byte),
            // Printable
            0x20..=0x7E => Action::Print(byte),
            // DEL — ignore
            0x7F => Action::None,
            // High bytes — treat as printable (will map to replacement glyph)
            _ => Action::Print(byte),
        }
    }

    fn escape(&mut self, byte: u8) -> Action {
        match byte {
            b'[' => {
                self.csi_reset();
                self.state = State::CsiParam;
                Action::None
            }
            b']' => {
                self.state = State::OscString;
                Action::None
            }
            // ESC c — full reset, treat as clear screen
            b'c' => {
                self.state = State::Ground;
                Action::CsiDispatch {
                    params: {
                        let mut p = [0u16; MAX_PARAMS];
                        p[0] = 2;
                        p
                    },
                    param_count: 1,
                    final_byte: b'J',
                }
            }
            // Any other byte after ESC — ignore the sequence and process the byte
            _ => {
                self.state = State::Ground;
                self.ground(byte)
            }
        }
    }

    fn csi_param(&mut self, byte: u8) -> Action {
        match byte {
            // Digit — accumulate into current parameter
            b'0'..=b'9' => {
                if !self.param_started {
                    self.param_started = true;
                    if self.param_count < MAX_PARAMS {
                        self.param_count += 1;
                    }
                }
                if self.param_count > 0 && self.param_count <= MAX_PARAMS {
                    let idx = self.param_count - 1;
                    self.params[idx] = self.params[idx]
                        .saturating_mul(10)
                        .saturating_add((byte - b'0') as u16);
                }
                Action::None
            }
            // Semicolon — parameter separator
            b';' => {
                if !self.param_started {
                    // Empty parameter before semicolon → default (0)
                    if self.param_count < MAX_PARAMS {
                        self.param_count += 1;
                    }
                }
                self.param_started = false;
                Action::None
            }
            // Intermediate bytes
            0x20..=0x2F => {
                self.state = State::CsiIntermediate;
                Action::None
            }
            // Final byte (0x40..=0x7E) — dispatch
            0x40..=0x7E => {
                self.state = State::Ground;
                Action::CsiDispatch {
                    params: self.params,
                    param_count: self.param_count,
                    final_byte: byte,
                }
            }
            // Anything else — abort sequence
            _ => {
                self.state = State::Ground;
                Action::None
            }
        }
    }

    fn csi_intermediate(&mut self, byte: u8) -> Action {
        match byte {
            // More intermediates
            0x20..=0x2F => Action::None,
            // Final byte — dispatch (we ignore intermediate bytes for now)
            0x40..=0x7E => {
                self.state = State::Ground;
                Action::CsiDispatch {
                    params: self.params,
                    param_count: self.param_count,
                    final_byte: byte,
                }
            }
            // Anything else — abort
            _ => {
                self.state = State::Ground;
                Action::None
            }
        }
    }

    fn osc_string(&mut self, byte: u8) -> Action {
        match byte {
            // ST (String Terminator) = BEL or ESC backslash
            0x07 => {
                self.state = State::Ground;
                Action::None
            }
            0x1B => {
                // Could be ESC \ (ST), but we just abort OSC on any ESC
                self.state = State::Escape;
                Action::None
            }
            // Consume all other bytes silently
            _ => Action::None,
        }
    }
}
