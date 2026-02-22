//! Framebuffer console (fbcon) — cell-based text console with ANSI color
//! and cursor support.
//!
//! Replaces the simple byte-at-a-time `DeviceFramebufferSink` with a proper
//! cell grid. Each character cell stores its glyph, foreground, and background
//! colors. An ANSI escape sequence parser processes SGR (color), cursor
//! movement, and erase commands.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;

use crate::driver_api::framebuffer::{Framebuffer, FramebufferInfo};
use crate::drivers::font_console::px16;
use crate::log::{LogLevel, LogSink};
use crate::sync::SpinLock;

pub mod ansi;
pub mod cell;
pub mod render;

use ansi::{Action, AnsiParser};
use cell::{AnsiColor, Cell, Color, DirtyBits};

// ---------------------------------------------------------------------------
// FbCon
// ---------------------------------------------------------------------------

/// Framebuffer console backed by a device-registry framebuffer.
pub struct FbCon {
    fb: Arc<dyn Framebuffer>,
    fb_info: FramebufferInfo,
    state: SpinLock<FbConState>,
}

struct FbConState {
    /// Character cell grid (cols * rows, row-major).
    cells: Box<[Cell]>,
    /// Tracks which cells need re-rendering.
    dirty: DirtyBits,
    /// Current cursor column.
    cursor_col: u32,
    /// Current cursor row.
    cursor_row: u32,
    /// Saved cursor column (ESC[s).
    saved_col: u32,
    /// Saved cursor row (ESC[u).
    saved_row: u32,
    /// Current foreground color for new characters.
    current_fg: Color,
    /// Current background color for new characters.
    current_bg: Color,
    /// ANSI escape sequence parser.
    parser: AnsiParser,
    /// Number of character columns.
    cols: u32,
    /// Number of character rows.
    rows: u32,
    /// Glyph width in pixels.
    glyph_width: u32,
    /// Glyph height in pixels.
    glyph_height: u32,
}

impl FbCon {
    /// Creates a new framebuffer console.
    ///
    /// The console dimensions are computed from the framebuffer size and the
    /// embedded font metrics.
    pub fn new(fb: Arc<dyn Framebuffer>) -> Self {
        let fb_info = fb.info();
        let glyph_width = px16::WIDTH;
        let glyph_height = px16::HEIGHT;
        let cols = fb_info.width / glyph_width;
        let rows = fb_info.height / glyph_height;

        let cell_count = (cols * rows) as usize;
        let cells = alloc::vec![Cell::BLANK; cell_count].into_boxed_slice();

        Self {
            fb,
            fb_info,
            state: SpinLock::named(
                "FBCON",
                FbConState {
                    cells,
                    dirty: DirtyBits::new(),
                    cursor_col: 0,
                    cursor_row: 0,
                    saved_col: 0,
                    saved_row: 0,
                    current_fg: Color::Default,
                    current_bg: Color::Default,
                    parser: AnsiParser::new(),
                    cols,
                    rows,
                    glyph_width,
                    glyph_height,
                },
            ),
        }
    }

    /// Writes a string to the console, processing ANSI escapes.
    pub fn write_str(&self, s: &str) {
        let mut state = self.state.lock();
        for byte in s.bytes() {
            let action = state.parser.feed(byte);
            self.apply_action(&mut state, action);
        }
        self.flush_dirty(&mut state);
    }

    /// Applies a single parser action to the console state.
    fn apply_action(&self, state: &mut FbConState, action: Action) {
        match action {
            Action::None => {}
            Action::Print(ch) => {
                self.put_char(state, ch);
            }
            Action::Execute(byte) => {
                self.execute_control(state, byte);
            }
            Action::CsiDispatch {
                params,
                param_count,
                final_byte,
            } => {
                self.csi_dispatch(state, &params[..param_count], final_byte);
            }
        }
    }

    /// Places a printable character at the cursor position, advancing the cursor.
    fn put_char(&self, state: &mut FbConState, ch: u8) {
        // Wrap to next line if at end of current line.
        if state.cursor_col >= state.cols {
            state.cursor_col = 0;
            state.cursor_row += 1;
        }

        // Scroll if past the last row.
        if state.cursor_row >= state.rows {
            self.scroll_up(state);
            state.cursor_row = state.rows - 1;
        }

        let idx = (state.cursor_row * state.cols + state.cursor_col) as usize;
        state.cells[idx] = Cell {
            ch,
            fg: state.current_fg,
            bg: state.current_bg,
        };
        state.dirty.set(idx as u32);
        state.cursor_col += 1;
    }

    /// Handles a C0 control character.
    fn execute_control(&self, state: &mut FbConState, byte: u8) {
        match byte {
            b'\n' => {
                state.cursor_col = 0;
                state.cursor_row += 1;
                if state.cursor_row >= state.rows {
                    self.scroll_up(state);
                    state.cursor_row = state.rows - 1;
                }
            }
            b'\r' => {
                state.cursor_col = 0;
            }
            0x08 => {
                // Backspace: move cursor back.
                if state.cursor_col > 0 {
                    state.cursor_col -= 1;
                } else if state.cursor_row > 0 {
                    state.cursor_row -= 1;
                    state.cursor_col = state.cols - 1;
                }
            }
            b'\t' => {
                let next = (state.cursor_col + 4) & !3;
                state.cursor_col = next.min(state.cols - 1);
            }
            _ => {}
        }
    }

    /// Dispatches a CSI escape sequence.
    fn csi_dispatch(&self, state: &mut FbConState, params: &[u16], final_byte: u8) {
        match final_byte {
            // SGR — Select Graphic Rendition
            b'm' => self.sgr(state, params),
            // CUP — Cursor Position
            b'H' | b'f' => {
                let row = param_or(params, 0, 1).saturating_sub(1);
                let col = param_or(params, 1, 1).saturating_sub(1);
                state.cursor_row = (row as u32).min(state.rows - 1);
                state.cursor_col = (col as u32).min(state.cols - 1);
            }
            // CUU — Cursor Up
            b'A' => {
                let n = param_or(params, 0, 1) as u32;
                state.cursor_row = state.cursor_row.saturating_sub(n);
            }
            // CUD — Cursor Down
            b'B' => {
                let n = param_or(params, 0, 1) as u32;
                state.cursor_row = (state.cursor_row + n).min(state.rows - 1);
            }
            // CUF — Cursor Forward
            b'C' => {
                let n = param_or(params, 0, 1) as u32;
                state.cursor_col = (state.cursor_col + n).min(state.cols - 1);
            }
            // CUB — Cursor Back
            b'D' => {
                let n = param_or(params, 0, 1) as u32;
                state.cursor_col = state.cursor_col.saturating_sub(n);
            }
            // ED — Erase in Display
            b'J' => {
                let mode = param_or(params, 0, 0);
                self.erase_display(state, mode);
            }
            // EL — Erase in Line
            b'K' => {
                let mode = param_or(params, 0, 0);
                self.erase_line(state, mode);
            }
            // SCP — Save Cursor Position
            b's' => {
                state.saved_col = state.cursor_col;
                state.saved_row = state.cursor_row;
            }
            // RCP — Restore Cursor Position
            b'u' => {
                state.cursor_col = state.saved_col;
                state.cursor_row = state.saved_row;
            }
            _ => {} // Unknown CSI — ignore
        }
    }

    /// Handles SGR (Select Graphic Rendition) parameters.
    fn sgr(&self, state: &mut FbConState, params: &[u16]) {
        // No params → reset
        if params.is_empty() {
            state.current_fg = Color::Default;
            state.current_bg = Color::Default;
            return;
        }

        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => {
                    state.current_fg = Color::Default;
                    state.current_bg = Color::Default;
                }
                1 => {
                    // Bold — brighten current fg if it's a standard color
                    if let Color::Ansi(c) = state.current_fg {
                        let bright = (c as u8) | 0x08;
                        if let Some(bc) = ansi_color_from_u8(bright) {
                            state.current_fg = Color::Ansi(bc);
                        }
                    } else {
                        state.current_fg = Color::Ansi(AnsiColor::BrightWhite);
                    }
                }
                7 => {
                    // Reverse video — swap fg and bg
                    core::mem::swap(&mut state.current_fg, &mut state.current_bg);
                }
                // Standard foreground colors (30-37)
                30..=37 => {
                    let idx = (params[i] - 30) as u8;
                    if let Some(c) = ansi_color_from_u8(idx) {
                        state.current_fg = Color::Ansi(c);
                    }
                }
                // Default foreground
                39 => {
                    state.current_fg = Color::Default;
                }
                // Standard background colors (40-47)
                40..=47 => {
                    let idx = (params[i] - 40) as u8;
                    if let Some(c) = ansi_color_from_u8(idx) {
                        state.current_bg = Color::Ansi(c);
                    }
                }
                // Default background
                49 => {
                    state.current_bg = Color::Default;
                }
                // Bright foreground colors (90-97)
                90..=97 => {
                    let idx = (params[i] - 90 + 8) as u8;
                    if let Some(c) = ansi_color_from_u8(idx) {
                        state.current_fg = Color::Ansi(c);
                    }
                }
                // Bright background colors (100-107)
                100..=107 => {
                    let idx = (params[i] - 100 + 8) as u8;
                    if let Some(c) = ansi_color_from_u8(idx) {
                        state.current_bg = Color::Ansi(c);
                    }
                }
                _ => {} // Unknown SGR param — ignore
            }
            i += 1;
        }
    }

    /// Erases part or all of the display.
    fn erase_display(&self, state: &mut FbConState, mode: u16) {
        let total = state.cols * state.rows;
        match mode {
            0 => {
                // Erase from cursor to end
                let start = state.cursor_row * state.cols + state.cursor_col;
                for i in start..total {
                    state.cells[i as usize] = Cell::BLANK;
                }
                state.dirty.set_range(start, total);
            }
            1 => {
                // Erase from start to cursor
                let end = state.cursor_row * state.cols + state.cursor_col + 1;
                for i in 0..end {
                    state.cells[i as usize] = Cell::BLANK;
                }
                state.dirty.set_range(0, end);
            }
            2 | 3 => {
                // Erase entire display
                for cell in state.cells.iter_mut() {
                    *cell = Cell::BLANK;
                }
                state.dirty.set_all(total);
                state.cursor_col = 0;
                state.cursor_row = 0;
            }
            _ => {}
        }
    }

    /// Erases part or all of the current line.
    fn erase_line(&self, state: &mut FbConState, mode: u16) {
        let row_start = state.cursor_row * state.cols;
        match mode {
            0 => {
                // Erase from cursor to end of line
                let start = row_start + state.cursor_col;
                let end = row_start + state.cols;
                for i in start..end {
                    state.cells[i as usize] = Cell::BLANK;
                }
                state.dirty.set_range(start, end);
            }
            1 => {
                // Erase from start of line to cursor
                let end = row_start + state.cursor_col + 1;
                for i in row_start..end {
                    state.cells[i as usize] = Cell::BLANK;
                }
                state.dirty.set_range(row_start, end);
            }
            2 => {
                // Erase entire line
                let end = row_start + state.cols;
                for i in row_start..end {
                    state.cells[i as usize] = Cell::BLANK;
                }
                state.dirty.set_range(row_start, end);
            }
            _ => {}
        }
    }

    /// Scrolls the entire screen up by one row.
    ///
    /// Uses `fb.copy_within()` for hardware-speed blit, then clears the
    /// bottom row in the cell grid and marks it dirty.
    fn scroll_up(&self, state: &mut FbConState) {
        let cols = state.cols;
        let rows = state.rows;
        let glyph_height = state.glyph_height;

        if rows <= 1 {
            return;
        }

        // Flush any pending dirty cells to pixels BEFORE the hardware blit,
        // so the blit operates on up-to-date pixel data.
        self.flush_dirty(state);

        // Blit framebuffer: shift all pixel rows up by one glyph height.
        let pitch = self.fb_info.pitch as usize;
        let row_bytes = pitch * glyph_height as usize;
        let src_offset = row_bytes as u64;
        let copy_count = row_bytes * (rows as usize - 1);

        // SAFETY: Scroll copies within the valid framebuffer region. The
        // source starts one glyph row in, destination is the top of the FB,
        // and the total copied bytes equal (rows-1) * row_bytes which is
        // within the FB mapping.
        unsafe {
            self.fb.copy_within(src_offset, 0, copy_count);
            self.fb
                .fill_zero((row_bytes * (rows as usize - 1)) as u64, row_bytes);
        }

        // Shift cell grid up by one row (memmove the cell array).
        let cell_row = cols as usize;
        state.cells.copy_within(cell_row.., 0);
        // Clear the last row.
        let last_row_start = (rows - 1) as usize * cell_row;
        for cell in &mut state.cells[last_row_start..] {
            *cell = Cell::BLANK;
        }

        // Only the last row needs re-rendering (the blit handled the rest).
        let last_row_idx = (rows - 1) * cols;
        state.dirty.set_range(last_row_idx, last_row_idx + cols);
    }

    /// Flushes dirty cells to the framebuffer.
    fn flush_dirty(&self, state: &mut FbConState) {
        let cols = state.cols;
        let glyph_width = state.glyph_width;
        let glyph_height = state.glyph_height;
        let fb = self.fb.as_ref();
        let info = &self.fb_info;

        state.dirty.drain(|idx| {
            let col = idx % cols;
            let row = idx / cols;
            render::render_cell(
                fb,
                info,
                col,
                row,
                &state.cells[idx as usize],
                glyph_width,
                glyph_height,
            );
        });
    }
}

// SAFETY: FbCon is Send+Sync because:
// - fb: Arc<dyn Framebuffer> is Send+Sync (trait bound)
// - fb_info: plain Copy data
// - state: SpinLock provides synchronized access
unsafe impl Send for FbCon {}
unsafe impl Sync for FbCon {}

// ---------------------------------------------------------------------------
// FbConSink — LogSink adapter
// ---------------------------------------------------------------------------

/// Wraps an [`FbCon`] to implement [`LogSink`].
pub struct FbConSink {
    fbcon: Arc<FbCon>,
    max_level: LogLevel,
}

impl FbConSink {
    /// Creates a new log sink backed by the given fbcon instance.
    pub fn new(fbcon: Arc<FbCon>, max_level: LogLevel) -> Self {
        Self { fbcon, max_level }
    }
}

impl LogSink for FbConSink {
    fn write_str(&self, s: &str) {
        self.fbcon.write_str(s);
    }

    fn max_level(&self) -> LogLevel {
        self.max_level
    }

    fn name(&self) -> &str {
        "framebuffer"
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `params[index]` if present and non-zero, otherwise `default`.
fn param_or(params: &[u16], index: usize, default: u16) -> u16 {
    params
        .get(index)
        .copied()
        .filter(|&v| v != 0)
        .unwrap_or(default)
}

/// Maps a `u8` index (0..15) to an `AnsiColor` variant.
fn ansi_color_from_u8(idx: u8) -> Option<AnsiColor> {
    match idx {
        0 => Some(AnsiColor::Black),
        1 => Some(AnsiColor::Red),
        2 => Some(AnsiColor::Green),
        3 => Some(AnsiColor::Yellow),
        4 => Some(AnsiColor::Blue),
        5 => Some(AnsiColor::Magenta),
        6 => Some(AnsiColor::Cyan),
        7 => Some(AnsiColor::White),
        8 => Some(AnsiColor::BrightBlack),
        9 => Some(AnsiColor::BrightRed),
        10 => Some(AnsiColor::BrightGreen),
        11 => Some(AnsiColor::BrightYellow),
        12 => Some(AnsiColor::BrightBlue),
        13 => Some(AnsiColor::BrightMagenta),
        14 => Some(AnsiColor::BrightCyan),
        15 => Some(AnsiColor::BrightWhite),
        _ => None,
    }
}
