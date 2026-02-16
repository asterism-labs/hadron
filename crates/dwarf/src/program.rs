//! DWARF line number program state machine.
//!
//! Implements the DWARF line number program as an iterator that yields
//! [`LineRow`] entries representing address-to-line mappings.

use crate::header::LineProgramHeader;
use crate::leb128::{decode_sleb128, decode_uleb128};

/// A row emitted by the line number program state machine.
#[derive(Debug, Clone, Copy)]
pub struct LineRow {
    /// Machine code address.
    pub address: u64,
    /// File index (1-based for DWARF v4, 0-based for DWARF v5).
    pub file_index: u64,
    /// Source line number (1-based).
    pub line: u32,
    /// Source column number (0 = unknown).
    pub column: u32,
    /// Whether this row is a recommended breakpoint location.
    pub is_stmt: bool,
    /// Whether this row marks the end of a sequence of addresses.
    pub end_sequence: bool,
}

// Standard opcodes
const DW_LNS_COPY: u8 = 1;
const DW_LNS_ADVANCE_PC: u8 = 2;
const DW_LNS_ADVANCE_LINE: u8 = 3;
const DW_LNS_SET_FILE: u8 = 4;
const DW_LNS_SET_COLUMN: u8 = 5;
const DW_LNS_NEGATE_STMT: u8 = 6;
const DW_LNS_FIXED_ADVANCE_PC: u8 = 9;
const DW_LNS_CONST_ADD_PC: u8 = 12;

// Extended opcode marker
const DW_LNE_MARKER: u8 = 0;
// Extended opcodes
const DW_LNE_END_SEQUENCE: u8 = 1;
const DW_LNE_SET_ADDRESS: u8 = 2;

/// Iterator over line number rows from a DWARF line program.
pub struct LineProgramIter<'a> {
    /// The bytecode to execute.
    bytecode: &'a [u8],
    /// Current position in the bytecode.
    cursor: usize,
    /// VM registers.
    address: u64,
    file: u64,
    line: i64,
    column: u32,
    is_stmt: bool,
    /// Header parameters needed for opcode decoding.
    line_base: i8,
    line_range: u8,
    opcode_base: u8,
    min_instruction_length: u8,
    max_ops_per_instruction: u8,
    default_is_stmt: bool,
    /// Standard opcode argument counts (for skipping unknown opcodes).
    standard_opcode_lengths: [u8; 24],
    /// Whether the iterator has finished.
    done: bool,
}

impl<'a> LineProgramIter<'a> {
    /// Creates a new line program iterator from header parameters and bytecode.
    pub(crate) fn new(header: &LineProgramHeader<'_>, bytecode: &'a [u8]) -> Self {
        Self {
            bytecode,
            cursor: 0,
            address: 0,
            file: if header.version >= 5 { 0 } else { 1 },
            line: 1,
            column: 0,
            is_stmt: header.default_is_stmt,
            line_base: header.line_base,
            line_range: header.line_range,
            opcode_base: header.opcode_base,
            min_instruction_length: header.minimum_instruction_length,
            max_ops_per_instruction: header.maximum_operations_per_instruction,
            default_is_stmt: header.default_is_stmt,
            standard_opcode_lengths: header.standard_opcode_lengths,
            done: false,
        }
    }

    /// Reads a single byte and advances the cursor.
    fn read_u8(&mut self) -> Option<u8> {
        if self.cursor >= self.bytecode.len() {
            return None;
        }
        let b = self.bytecode[self.cursor];
        self.cursor += 1;
        Some(b)
    }

    /// Reads a little-endian u16 and advances the cursor.
    fn read_u16(&mut self) -> Option<u16> {
        if self.cursor + 2 > self.bytecode.len() {
            return None;
        }
        let v = u16::from_le_bytes([
            self.bytecode[self.cursor],
            self.bytecode[self.cursor + 1],
        ]);
        self.cursor += 2;
        Some(v)
    }

    /// Reads a ULEB128 and advances the cursor.
    fn read_uleb128(&mut self) -> Option<u64> {
        let (v, consumed) = decode_uleb128(&self.bytecode[self.cursor..])?;
        self.cursor += consumed;
        Some(v)
    }

    /// Reads a SLEB128 and advances the cursor.
    fn read_sleb128(&mut self) -> Option<i64> {
        let (v, consumed) = decode_sleb128(&self.bytecode[self.cursor..])?;
        self.cursor += consumed;
        Some(v)
    }

    /// Reads a target-sized address (8 bytes for 64-bit).
    fn read_address(&mut self) -> Option<u64> {
        if self.cursor + 8 > self.bytecode.len() {
            return None;
        }
        let v = u64::from_le_bytes([
            self.bytecode[self.cursor],
            self.bytecode[self.cursor + 1],
            self.bytecode[self.cursor + 2],
            self.bytecode[self.cursor + 3],
            self.bytecode[self.cursor + 4],
            self.bytecode[self.cursor + 5],
            self.bytecode[self.cursor + 6],
            self.bytecode[self.cursor + 7],
        ]);
        self.cursor += 8;
        Some(v)
    }

    /// Creates a `LineRow` snapshot of the current VM state.
    fn emit_row(&self, end_sequence: bool) -> LineRow {
        LineRow {
            address: self.address,
            file_index: self.file,
            line: self.line.max(0) as u32,
            column: self.column,
            is_stmt: self.is_stmt,
            end_sequence,
        }
    }

    /// Resets the VM registers to their initial values (after end_sequence).
    fn reset_registers(&mut self) {
        self.address = 0;
        self.file = 1;
        self.line = 1;
        self.column = 0;
        self.is_stmt = self.default_is_stmt;
    }

    /// Computes the address advance for a given operation advance.
    fn advance_address(&self, op_advance: u64) -> u64 {
        let max_ops = u64::from(self.max_ops_per_instruction).max(1);
        let min_len = u64::from(self.min_instruction_length);
        (op_advance / max_ops) * min_len
    }
}

impl<'a> Iterator for LineProgramIter<'a> {
    type Item = LineRow;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.done || self.cursor >= self.bytecode.len() {
                return None;
            }

            let opcode = self.read_u8()?;

            if opcode == DW_LNE_MARKER {
                // Extended opcode
                let length = self.read_uleb128()? as usize;
                if length == 0 || self.cursor >= self.bytecode.len() {
                    return None;
                }
                let ext_opcode = self.read_u8()?;
                let ext_data_len = length.saturating_sub(1);

                match ext_opcode {
                    DW_LNE_END_SEQUENCE => {
                        let row = self.emit_row(true);
                        self.reset_registers();
                        return Some(row);
                    }
                    DW_LNE_SET_ADDRESS => {
                        self.address = self.read_address()?;
                    }
                    _ => {
                        // Skip unknown extended opcode
                        if self.cursor + ext_data_len > self.bytecode.len() {
                            self.done = true;
                            return None;
                        }
                        self.cursor += ext_data_len;
                    }
                }
            } else if opcode < self.opcode_base {
                // Standard opcode
                match opcode {
                    DW_LNS_COPY => {
                        let row = self.emit_row(false);
                        return Some(row);
                    }
                    DW_LNS_ADVANCE_PC => {
                        let advance = self.read_uleb128()?;
                        self.address += self.advance_address(advance);
                    }
                    DW_LNS_ADVANCE_LINE => {
                        let delta = self.read_sleb128()?;
                        self.line = self.line.wrapping_add(delta);
                    }
                    DW_LNS_SET_FILE => {
                        self.file = self.read_uleb128()?;
                    }
                    DW_LNS_SET_COLUMN => {
                        self.column = self.read_uleb128()? as u32;
                    }
                    DW_LNS_NEGATE_STMT => {
                        self.is_stmt = !self.is_stmt;
                    }
                    DW_LNS_FIXED_ADVANCE_PC => {
                        let advance = self.read_u16()?;
                        self.address += u64::from(advance);
                    }
                    DW_LNS_CONST_ADD_PC => {
                        let adjusted = u64::from(255 - self.opcode_base) / u64::from(self.line_range);
                        self.address += self.advance_address(adjusted);
                    }
                    _ => {
                        // Unknown standard opcode — skip its arguments
                        let idx = (opcode as usize).saturating_sub(1);
                        let argc = if idx < self.standard_opcode_lengths.len() {
                            self.standard_opcode_lengths[idx]
                        } else {
                            0
                        };
                        for _ in 0..argc {
                            self.read_uleb128()?;
                        }
                    }
                }
            } else {
                // Special opcode
                let adjusted = opcode - self.opcode_base;
                let op_advance = u64::from(adjusted) / u64::from(self.line_range);
                let line_inc = i64::from(self.line_base)
                    + i64::from(adjusted % self.line_range);

                self.address += self.advance_address(op_advance);
                self.line = self.line.wrapping_add(line_inc);

                let row = self.emit_row(false);
                return Some(row);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::LineProgramHeader;

    /// Build a minimal DWARF v4 line program with specific bytecode.
    fn make_program_with_bytecode(bytecode: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();

        // unit_length placeholder
        buf.extend_from_slice(&[0u8; 4]);

        // Version: 4
        buf.extend_from_slice(&4u16.to_le_bytes());

        // header_length placeholder
        let header_length_pos = buf.len();
        buf.extend_from_slice(&[0u8; 4]);

        let header_start = buf.len();

        // minimum_instruction_length: 1
        buf.push(1);
        // maximum_operations_per_instruction: 1
        buf.push(1);
        // default_is_stmt: 1
        buf.push(1);
        // line_base: -5
        buf.push((-5i8) as u8);
        // line_range: 14
        buf.push(14);
        // opcode_base: 13
        buf.push(13);
        // standard_opcode_lengths (12 entries)
        buf.extend_from_slice(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]);

        // Empty directory table
        buf.push(0);
        // Empty file table
        buf.push(0);

        let header_end = buf.len();
        let header_length = (header_end - header_start) as u32;
        buf[header_length_pos..header_length_pos + 4]
            .copy_from_slice(&header_length.to_le_bytes());

        // Append the bytecode
        buf.extend_from_slice(bytecode);

        // Fix unit_length
        let unit_length = (buf.len() - 4) as u32;
        buf[0..4].copy_from_slice(&unit_length.to_le_bytes());

        buf
    }

    #[test]
    fn end_sequence_only() {
        let bytecode = &[
            0x00, 0x01, 0x01, // DW_LNE_end_sequence (extended: marker=0, len=1, opcode=1)
        ];
        let buf = make_program_with_bytecode(bytecode);
        let header = LineProgramHeader::parse(&buf).unwrap();
        let rows: Vec<_> = header.rows(&buf).collect();

        assert_eq!(rows.len(), 1);
        assert!(rows[0].end_sequence);
        assert_eq!(rows[0].address, 0);
        assert_eq!(rows[0].line, 1);
    }

    #[test]
    fn set_address_and_copy() {
        let mut bytecode = Vec::new();
        // DW_LNE_set_address: extended marker=0, len=9, opcode=2, addr=0x1000
        bytecode.push(0x00);
        bytecode.push(9); // length (1 opcode byte + 8 address bytes)
        bytecode.push(0x02); // DW_LNE_set_address
        bytecode.extend_from_slice(&0x1000u64.to_le_bytes());
        // DW_LNS_copy
        bytecode.push(0x01);
        // DW_LNE_end_sequence
        bytecode.push(0x00);
        bytecode.push(0x01);
        bytecode.push(0x01);

        let buf = make_program_with_bytecode(&bytecode);
        let header = LineProgramHeader::parse(&buf).unwrap();
        let rows: Vec<_> = header.rows(&buf).collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].address, 0x1000);
        assert_eq!(rows[0].line, 1);
        assert!(!rows[0].end_sequence);
        assert!(rows[1].end_sequence);
    }

    #[test]
    fn advance_line_and_pc() {
        let mut bytecode = Vec::new();
        // DW_LNE_set_address to 0x2000
        bytecode.push(0x00);
        bytecode.push(9);
        bytecode.push(0x02);
        bytecode.extend_from_slice(&0x2000u64.to_le_bytes());
        // DW_LNS_advance_line by 9 (SLEB128: 0x09)
        bytecode.push(0x03); // DW_LNS_advance_line
        bytecode.push(0x09); // +9 lines (line becomes 10)
        // DW_LNS_advance_pc by 16 (ULEB128: 0x10)
        bytecode.push(0x02); // DW_LNS_advance_pc
        bytecode.push(0x10); // +16 bytes
        // DW_LNS_copy
        bytecode.push(0x01);
        // DW_LNE_end_sequence
        bytecode.push(0x00);
        bytecode.push(0x01);
        bytecode.push(0x01);

        let buf = make_program_with_bytecode(&bytecode);
        let header = LineProgramHeader::parse(&buf).unwrap();
        let rows: Vec<_> = header.rows(&buf).collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].address, 0x2000 + 16);
        assert_eq!(rows[0].line, 10);
    }

    #[test]
    fn special_opcode() {
        let mut bytecode = Vec::new();
        // DW_LNE_set_address to 0x3000
        bytecode.push(0x00);
        bytecode.push(9);
        bytecode.push(0x02);
        bytecode.extend_from_slice(&0x3000u64.to_le_bytes());
        // Special opcode: opcode_base=13, line_base=-5, line_range=14
        // adjusted = opcode - opcode_base
        // line_inc = line_base + (adjusted % line_range)
        // op_advance = adjusted / line_range
        // For opcode 20: adjusted=7, op_advance=0, line_inc=-5+7=2
        bytecode.push(20);
        // DW_LNE_end_sequence
        bytecode.push(0x00);
        bytecode.push(0x01);
        bytecode.push(0x01);

        let buf = make_program_with_bytecode(&bytecode);
        let header = LineProgramHeader::parse(&buf).unwrap();
        let rows: Vec<_> = header.rows(&buf).collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].address, 0x3000); // op_advance=0
        assert_eq!(rows[0].line, 3); // 1 + 2
    }

    #[test]
    fn set_file_and_column() {
        let mut bytecode = Vec::new();
        // DW_LNS_set_file to 3
        bytecode.push(0x04);
        bytecode.push(0x03);
        // DW_LNS_set_column to 42
        bytecode.push(0x05);
        bytecode.push(42);
        // DW_LNS_copy
        bytecode.push(0x01);
        // DW_LNE_end_sequence
        bytecode.push(0x00);
        bytecode.push(0x01);
        bytecode.push(0x01);

        let buf = make_program_with_bytecode(&bytecode);
        let header = LineProgramHeader::parse(&buf).unwrap();
        let rows: Vec<_> = header.rows(&buf).collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].file_index, 3);
        assert_eq!(rows[0].column, 42);
    }

    #[test]
    fn negate_stmt() {
        let mut bytecode = Vec::new();
        // DW_LNS_negate_stmt
        bytecode.push(0x06);
        // DW_LNS_copy
        bytecode.push(0x01);
        // DW_LNE_end_sequence
        bytecode.push(0x00);
        bytecode.push(0x01);
        bytecode.push(0x01);

        let buf = make_program_with_bytecode(&bytecode);
        let header = LineProgramHeader::parse(&buf).unwrap();
        let rows: Vec<_> = header.rows(&buf).collect();

        assert_eq!(rows.len(), 2);
        assert!(!rows[0].is_stmt); // was true (default), negated to false
    }

    #[test]
    fn fixed_advance_pc() {
        let mut bytecode = Vec::new();
        // DW_LNS_fixed_advance_pc by 0x100
        bytecode.push(0x09);
        bytecode.extend_from_slice(&0x100u16.to_le_bytes());
        // DW_LNS_copy
        bytecode.push(0x01);
        // DW_LNE_end_sequence
        bytecode.push(0x00);
        bytecode.push(0x01);
        bytecode.push(0x01);

        let buf = make_program_with_bytecode(&bytecode);
        let header = LineProgramHeader::parse(&buf).unwrap();
        let rows: Vec<_> = header.rows(&buf).collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].address, 0x100);
    }

    #[test]
    fn const_add_pc() {
        let mut bytecode = Vec::new();
        // DW_LNS_const_add_pc
        bytecode.push(DW_LNS_CONST_ADD_PC);
        // DW_LNS_copy
        bytecode.push(0x01);
        // DW_LNE_end_sequence
        bytecode.push(0x00);
        bytecode.push(0x01);
        bytecode.push(0x01);

        let buf = make_program_with_bytecode(&bytecode);
        let header = LineProgramHeader::parse(&buf).unwrap();
        let rows: Vec<_> = header.rows(&buf).collect();

        // Expected: (255 - 13) / 14 = 17 (integer division)
        // advance_address(17) = 17 * 1 = 17
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].address, 17);
    }
}
