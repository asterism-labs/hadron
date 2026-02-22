//! Cell, color, and dirty-tracking types for the framebuffer console.

/// Standard ANSI color indices (0..15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AnsiColor {
    Black = 0,
    Red = 1,
    Green = 2,
    Yellow = 3,
    Blue = 4,
    Magenta = 5,
    Cyan = 6,
    White = 7,
    BrightBlack = 8,
    BrightRed = 9,
    BrightGreen = 10,
    BrightYellow = 11,
    BrightBlue = 12,
    BrightMagenta = 13,
    BrightCyan = 14,
    BrightWhite = 15,
}

/// BGR32 palette for the 16 standard ANSI colors.
///
/// Format: `0x00_RR_GG_BB` stored as a u32 in BGR32 (Bochs VGA native).
/// The byte layout in memory is `[B, G, R, 0]`.
static ANSI_PALETTE: [u32; 16] = [
    0x00_00_00_00, // 0  Black
    0x00_00_00_AA, // 1  Red
    0x00_00_AA_00, // 2  Green
    0x00_00_AA_AA, // 3  Yellow (dark)
    0x00_AA_00_00, // 4  Blue
    0x00_AA_00_AA, // 5  Magenta
    0x00_AA_AA_00, // 6  Cyan
    0x00_AA_AA_AA, // 7  White (light grey)
    0x00_55_55_55, // 8  Bright black (dark grey)
    0x00_55_55_FF, // 9  Bright red
    0x00_55_FF_55, // 10 Bright green
    0x00_55_FF_FF, // 11 Bright yellow
    0x00_FF_55_55, // 12 Bright blue
    0x00_FF_55_FF, // 13 Bright magenta
    0x00_FF_FF_55, // 14 Bright cyan
    0x00_FF_FF_FF, // 15 Bright white
];

/// Cell foreground/background color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// One of the 16 standard ANSI colors.
    Ansi(AnsiColor),
    /// Default terminal color (light grey fg / black bg).
    Default,
}

impl Color {
    /// Converts this color to a BGR32 pixel value.
    ///
    /// `is_foreground` selects the default: light grey for fg, black for bg.
    pub fn to_bgr32(self, is_foreground: bool) -> u32 {
        match self {
            Color::Ansi(c) => ANSI_PALETTE[c as usize],
            Color::Default => {
                if is_foreground {
                    ANSI_PALETTE[AnsiColor::White as usize] // 0xAAAAAA
                } else {
                    ANSI_PALETTE[AnsiColor::Black as usize] // 0x000000
                }
            }
        }
    }
}

/// A single character cell in the console grid.
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    /// ASCII character (or space for empty cells).
    pub ch: u8,
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
}

impl Cell {
    /// A blank cell with default colors.
    pub const BLANK: Self = Self {
        ch: b' ',
        fg: Color::Default,
        bg: Color::Default,
    };
}

/// Fixed-size bitset for tracking dirty cells.
///
/// Supports up to `MAX_WORDS * 64` cells. At 1920x1080 with 8x16 glyphs
/// that's 240x67 = 16,080 cells, needing 252 words.
pub struct DirtyBits {
    words: [u64; Self::MAX_WORDS],
}

impl DirtyBits {
    /// Maximum number of u64 words (supports up to ~16,384 cells).
    const MAX_WORDS: usize = 256;

    /// Creates an all-clean bitset.
    pub const fn new() -> Self {
        Self {
            words: [0u64; Self::MAX_WORDS],
        }
    }

    /// Marks a cell index as dirty.
    #[inline]
    pub fn set(&mut self, index: u32) {
        let i = index as usize;
        let word = i / 64;
        let bit = i % 64;
        if word < Self::MAX_WORDS {
            self.words[word] |= 1u64 << bit;
        }
    }

    /// Marks a range of cells as dirty (inclusive start, exclusive end).
    pub fn set_range(&mut self, start: u32, end: u32) {
        for i in start..end {
            self.set(i);
        }
    }

    /// Marks all cells as dirty up to `count`.
    pub fn set_all(&mut self, count: u32) {
        let full_words = (count as usize) / 64;
        let remainder = (count as usize) % 64;
        for w in self.words.iter_mut().take(full_words.min(Self::MAX_WORDS)) {
            *w = u64::MAX;
        }
        if full_words < Self::MAX_WORDS && remainder > 0 {
            self.words[full_words] = (1u64 << remainder) - 1;
        }
    }

    /// Clears all dirty bits.
    pub fn clear_all(&mut self) {
        self.words = [0u64; Self::MAX_WORDS];
    }

    /// Iterates over dirty cell indices and clears them.
    ///
    /// Calls `f(index)` for each dirty cell, then resets all bits to clean.
    pub fn drain(&mut self, mut f: impl FnMut(u32)) {
        for (word_idx, word) in self.words.iter_mut().enumerate() {
            let mut w = *word;
            while w != 0 {
                let bit = w.trailing_zeros();
                let index = (word_idx * 64 + bit as usize) as u32;
                f(index);
                w &= w - 1; // clear lowest set bit
            }
            *word = 0;
        }
    }
}
