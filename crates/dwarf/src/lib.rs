//! Minimal DWARF `.debug_line` parser for Hadron OS.
//!
//! Provides zero-copy, zero-allocation parsing of DWARF line number programs
//! from `.debug_line` sections. Supports DWARF v4 and v5 line program formats.
//!
//! # Usage
//!
//! ```
//! use hadron_dwarf::DebugLine;
//!
//! fn parse_lines(debug_line_data: &[u8]) {
//!     for unit in DebugLine::new(debug_line_data) {
//!         let header = unit.header();
//!         for row in unit.rows() {
//!             let file = header.file(row.file_index);
//!             // row.address, row.line, file.name, etc.
//!         }
//!     }
//! }
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod header;
pub mod leb128;
pub mod program;

pub use header::{DwarfError, FileEntry, LineProgramHeader};
pub use program::{LineProgramIter, LineRow};

/// Iterator over compilation unit line programs within a `.debug_line` section.
///
/// Each `.debug_line` section may contain multiple compilation units, each with
/// its own line program header and bytecode. This iterator yields one
/// [`LineProgramUnit`] per compilation unit.
pub struct DebugLine<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> DebugLine<'a> {
    /// Creates a new iterator over line programs in the given `.debug_line` data.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }
}

impl<'a> Iterator for DebugLine<'a> {
    type Item = LineProgramUnit<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.data.len() {
            return None;
        }
        let remaining = &self.data[self.offset..];
        // Need at least 4 bytes for unit_length
        if remaining.len() < 4 {
            return None;
        }
        let unit_length =
            u32::from_le_bytes([remaining[0], remaining[1], remaining[2], remaining[3]]);
        if unit_length == 0 {
            return None;
        }
        // Total unit size = 4 (length field) + unit_length
        let total = 4 + unit_length as usize;
        if total > remaining.len() {
            return None;
        }
        let unit_data = &remaining[..total];
        self.offset += total;

        let header = match LineProgramHeader::parse(unit_data) {
            Ok(h) => h,
            Err(_) => return self.next(), // skip malformed units
        };

        Some(LineProgramUnit {
            header,
            unit_data,
        })
    }
}

/// A single compilation unit's line program within a `.debug_line` section.
pub struct LineProgramUnit<'a> {
    header: LineProgramHeader<'a>,
    unit_data: &'a [u8],
}

impl<'a> LineProgramUnit<'a> {
    /// Returns a reference to the parsed line program header.
    #[must_use]
    pub fn header(&self) -> &LineProgramHeader<'a> {
        &self.header
    }

    /// Returns an iterator over the line number rows in this unit's program.
    #[must_use]
    pub fn rows(&self) -> LineProgramIter<'a> {
        self.header.rows(self.unit_data)
    }
}
