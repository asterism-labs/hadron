//! Terminal cell grid with ANSI escape sequence processing.
//!
//! Maintains a 2D grid of [`Cell`]s, a cursor position, and current text
//! attributes. Bytes from the PTY master are fed through an [`AnsiParser`]
//! which dispatches print, execute, and CSI actions onto the grid.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use crate::ansi::{Action, AnsiParser};

/// A single terminal cell.
#[derive(Clone, Copy)]
pub struct Cell {
    /// ASCII character (space for empty cells).
    pub ch: u8,
    /// Foreground color (0x00RRGGBB).
    pub fg: u32,
    /// Background color (0x00RRGGBB).
    pub bg: u32,
}

impl Cell {
    const fn blank() -> Self {
        Self {
            ch: b' ',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
        }
    }
}

/// Default foreground: light grey.
const DEFAULT_FG: u32 = 0x00CC_CCCC;
/// Default background: near-black.
const DEFAULT_BG: u32 = 0x0018_1818;

/// ANSI 8-color palette (standard colors).
const ANSI_COLORS: [u32; 8] = [
    0x0000_0000, // 0: black
    0x00CC_0000, // 1: red
    0x0000_CC00, // 2: green
    0x00CC_CC00, // 3: yellow
    0x0000_00CC, // 4: blue
    0x00CC_00CC, // 5: magenta
    0x0000_CCCC, // 6: cyan
    0x00CC_CCCC, // 7: white
];

/// Terminal cell grid.
pub struct Grid {
    /// Flat cell buffer (row-major: index = row * cols + col).
    pub cells: Vec<Cell>,
    /// Number of columns.
    pub cols: usize,
    /// Number of rows.
    pub rows: usize,
    /// Cursor column.
    pub cursor_col: usize,
    /// Cursor row.
    pub cursor_row: usize,
    /// Current foreground color for new characters.
    cur_fg: u32,
    /// Current background color for new characters.
    cur_bg: u32,
    /// ANSI escape sequence parser.
    parser: AnsiParser,
    /// Whether the grid has changed since last render.
    pub dirty: bool,
}

impl Grid {
    /// Create a new grid with the given dimensions.
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cells: vec![Cell::blank(); cols * rows],
            cols,
            rows,
            cursor_col: 0,
            cursor_row: 0,
            cur_fg: DEFAULT_FG,
            cur_bg: DEFAULT_BG,
            parser: AnsiParser::new(),
            dirty: true,
        }
    }

    /// Feed raw bytes from the PTY into the grid.
    pub fn feed_bytes(&mut self, data: &[u8]) {
        for &byte in data {
            let action = self.parser.feed(byte);
            self.dispatch(action);
        }
    }

    /// Dispatch a parsed ANSI action onto the grid.
    fn dispatch(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Print(ch) => self.put_char(ch),
            Action::Execute(byte) => self.execute(byte),
            Action::CsiDispatch {
                params,
                param_count,
                final_byte,
            } => self.csi_dispatch(&params, param_count, final_byte),
        }
    }

    /// Place a character at the cursor and advance.
    fn put_char(&mut self, ch: u8) {
        let idx = self.cursor_row * self.cols + self.cursor_col;
        if idx < self.cells.len() {
            self.cells[idx] = Cell {
                ch,
                fg: self.cur_fg,
                bg: self.cur_bg,
            };
        }
        self.cursor_col += 1;
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= self.rows {
                self.scroll_up();
                self.cursor_row = self.rows - 1;
            }
        }
        self.dirty = true;
    }

    /// Handle C0 control characters.
    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.cursor_row += 1;
                if self.cursor_row >= self.rows {
                    self.scroll_up();
                    self.cursor_row = self.rows - 1;
                }
            }
            b'\r' => {
                self.cursor_col = 0;
            }
            0x08 => {
                // Backspace: move cursor left.
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
            }
            0x09 => {
                // Tab: advance to next 8-column boundary.
                self.cursor_col = (self.cursor_col + 8) & !7;
                if self.cursor_col >= self.cols {
                    self.cursor_col = self.cols - 1;
                }
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// Handle CSI dispatch commands.
    fn csi_dispatch(&mut self, params: &[u16], param_count: usize, final_byte: u8) {
        // Helper to get parameter with a default value.
        let p = |idx: usize, def: u16| -> u16 {
            if idx < param_count && params[idx] != 0 {
                params[idx]
            } else {
                def
            }
        };

        match final_byte {
            // SGR — Select Graphic Rendition
            b'm' => {
                if param_count == 0 {
                    self.cur_fg = DEFAULT_FG;
                    self.cur_bg = DEFAULT_BG;
                    return;
                }
                for i in 0..param_count {
                    match params[i] {
                        0 => {
                            self.cur_fg = DEFAULT_FG;
                            self.cur_bg = DEFAULT_BG;
                        }
                        1 => {} // Bold — ignore for now
                        30..=37 => {
                            self.cur_fg = ANSI_COLORS[(params[i] - 30) as usize];
                        }
                        39 => self.cur_fg = DEFAULT_FG,
                        40..=47 => {
                            self.cur_bg = ANSI_COLORS[(params[i] - 40) as usize];
                        }
                        49 => self.cur_bg = DEFAULT_BG,
                        _ => {}
                    }
                }
            }
            // CUP — Cursor Position (CSI row;col H or f)
            b'H' | b'f' => {
                let row = p(0, 1).saturating_sub(1) as usize;
                let col = p(1, 1).saturating_sub(1) as usize;
                self.cursor_row = row.min(self.rows - 1);
                self.cursor_col = col.min(self.cols - 1);
            }
            // ED — Erase in Display
            b'J' => {
                let mode = p(0, 0);
                match mode {
                    0 => {
                        // Clear from cursor to end.
                        let start = self.cursor_row * self.cols + self.cursor_col;
                        for cell in &mut self.cells[start..] {
                            *cell = Cell::blank();
                        }
                    }
                    1 => {
                        // Clear from start to cursor.
                        let end = self.cursor_row * self.cols + self.cursor_col + 1;
                        let end = end.min(self.cells.len());
                        for cell in &mut self.cells[..end] {
                            *cell = Cell::blank();
                        }
                    }
                    2 => {
                        // Clear entire screen.
                        for cell in &mut self.cells {
                            *cell = Cell::blank();
                        }
                    }
                    _ => {}
                }
            }
            // EL — Erase in Line
            b'K' => {
                let mode = p(0, 0);
                let row_start = self.cursor_row * self.cols;
                match mode {
                    0 => {
                        // Clear from cursor to end of line.
                        let start = row_start + self.cursor_col;
                        let end = (row_start + self.cols).min(self.cells.len());
                        for cell in &mut self.cells[start..end] {
                            *cell = Cell::blank();
                        }
                    }
                    1 => {
                        // Clear from start of line to cursor.
                        let end = (row_start + self.cursor_col + 1).min(self.cells.len());
                        for cell in &mut self.cells[row_start..end] {
                            *cell = Cell::blank();
                        }
                    }
                    2 => {
                        // Clear entire line.
                        let end = (row_start + self.cols).min(self.cells.len());
                        for cell in &mut self.cells[row_start..end] {
                            *cell = Cell::blank();
                        }
                    }
                    _ => {}
                }
            }
            // CUU — Cursor Up
            b'A' => {
                let n = p(0, 1) as usize;
                self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            // CUD — Cursor Down
            b'B' => {
                let n = p(0, 1) as usize;
                self.cursor_row = (self.cursor_row + n).min(self.rows - 1);
            }
            // CUF — Cursor Forward
            b'C' => {
                let n = p(0, 1) as usize;
                self.cursor_col = (self.cursor_col + n).min(self.cols - 1);
            }
            // CUB — Cursor Back
            b'D' => {
                let n = p(0, 1) as usize;
                self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// Scroll the grid up by one line, clearing the bottom row.
    fn scroll_up(&mut self) {
        let row_bytes = self.cols;
        self.cells.copy_within(row_bytes.., 0);
        let start = (self.rows - 1) * self.cols;
        for cell in &mut self.cells[start..] {
            *cell = Cell::blank();
        }
        self.dirty = true;
    }
}
