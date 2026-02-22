//! DWARF line program header parsing.
//!
//! Parses the header of a `.debug_line` compilation unit, including the
//! directory table and file table. Supports DWARF v4 and v5 formats.

use crate::leb128::decode_uleb128;
use crate::program::LineProgramIter;

/// Maximum number of directories in a line program header.
const MAX_DIRECTORIES: usize = 256;

/// Maximum number of file entries in a line program header.
const MAX_FILES: usize = 1024;

/// Maximum number of standard opcode length entries.
const MAX_STANDARD_OPCODE_LENGTHS: usize = 24;

/// Errors that can occur when parsing DWARF data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DwarfError {
    /// The input data is too short for the declared structure.
    Truncated,
    /// An unsupported DWARF version was encountered.
    UnsupportedVersion,
    /// A header offset or size is out of bounds.
    InvalidOffset,
    /// A string in the line program is not valid UTF-8.
    InvalidUtf8,
    /// Too many directories in the line program header.
    TooManyDirectories,
    /// Too many file entries in the line program header.
    TooManyFiles,
}

impl core::fmt::Display for DwarfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated => write!(f, "DWARF data truncated"),
            Self::UnsupportedVersion => write!(f, "unsupported DWARF version"),
            Self::InvalidOffset => write!(f, "invalid DWARF offset"),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 in DWARF data"),
            Self::TooManyDirectories => write!(f, "too many directories in line program"),
            Self::TooManyFiles => write!(f, "too many files in line program"),
        }
    }
}

/// A file entry from the line program header.
#[derive(Debug, Clone, Copy)]
pub struct FileEntry<'a> {
    /// Index into the directory table.
    pub directory_index: u64,
    /// File name (zero-copy reference into the input data).
    pub name: &'a str,
}

/// A parsed DWARF line program header.
#[derive(Debug)]
pub struct LineProgramHeader<'a> {
    /// DWARF version (4 or 5).
    pub(crate) version: u16,
    /// Minimum instruction length.
    pub(crate) minimum_instruction_length: u8,
    /// Maximum operations per instruction.
    pub(crate) maximum_operations_per_instruction: u8,
    /// Default value of the `is_stmt` register.
    pub(crate) default_is_stmt: bool,
    /// Line base for special opcodes.
    pub(crate) line_base: i8,
    /// Line range for special opcodes.
    pub(crate) line_range: u8,
    /// First special opcode number.
    pub(crate) opcode_base: u8,
    /// Number of arguments for each standard opcode.
    pub(crate) standard_opcode_lengths: [u8; MAX_STANDARD_OPCODE_LENGTHS],
    /// Directory table.
    directories: [Option<&'a str>; MAX_DIRECTORIES],
    /// Number of directories.
    dir_count: usize,
    /// File table.
    files: [Option<FileEntry<'a>>; MAX_FILES],
    /// Number of files.
    file_count: usize,
    /// Byte offset where the line program bytecode starts (relative to unit start).
    pub(crate) program_offset: usize,
    /// Total unit length (including the 4-byte length field).
    pub(crate) unit_length: usize,
}

impl<'a> LineProgramHeader<'a> {
    /// Parse a line program header from the start of a compilation unit.
    ///
    /// `data` should start at the beginning of a `.debug_line` unit
    /// (i.e., at the `unit_length` field).
    ///
    /// # Errors
    ///
    /// Returns [`DwarfError`] if the data is malformed or uses an unsupported version.
    pub fn parse(data: &'a [u8]) -> Result<Self, DwarfError> {
        if data.len() < 4 {
            return Err(DwarfError::Truncated);
        }

        let unit_length = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let total_length = 4 + unit_length;
        if total_length > data.len() {
            return Err(DwarfError::Truncated);
        }

        let mut offset = 4;

        // Version (u16)
        if offset + 2 > data.len() {
            return Err(DwarfError::Truncated);
        }
        let version = u16::from_le_bytes([data[offset], data[offset + 1]]);
        offset += 2;

        if version < 4 || version > 5 {
            return Err(DwarfError::UnsupportedVersion);
        }

        // DWARF v5 has address_size and segment_selector_size before header_length
        if version >= 5 {
            if offset + 2 > data.len() {
                return Err(DwarfError::Truncated);
            }
            // address_size (u8) + segment_selector_size (u8)
            offset += 2;
        }

        // header_length (u32 for DWARF32)
        if offset + 4 > data.len() {
            return Err(DwarfError::Truncated);
        }
        let header_length = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;

        let program_offset = offset + header_length;
        if program_offset > total_length {
            return Err(DwarfError::InvalidOffset);
        }

        // minimum_instruction_length (u8)
        if offset >= data.len() {
            return Err(DwarfError::Truncated);
        }
        let minimum_instruction_length = data[offset];
        offset += 1;

        // maximum_operations_per_instruction (u8) — present in v4+
        if offset >= data.len() {
            return Err(DwarfError::Truncated);
        }
        let maximum_operations_per_instruction = data[offset];
        offset += 1;

        // default_is_stmt (u8)
        if offset >= data.len() {
            return Err(DwarfError::Truncated);
        }
        let default_is_stmt = data[offset] != 0;
        offset += 1;

        // line_base (i8)
        if offset >= data.len() {
            return Err(DwarfError::Truncated);
        }
        let line_base = data[offset] as i8;
        offset += 1;

        // line_range (u8)
        if offset >= data.len() {
            return Err(DwarfError::Truncated);
        }
        let line_range = data[offset];
        offset += 1;

        // opcode_base (u8)
        if offset >= data.len() {
            return Err(DwarfError::Truncated);
        }
        let opcode_base = data[offset];
        offset += 1;

        // standard_opcode_lengths array (opcode_base - 1 entries)
        let mut standard_opcode_lengths = [0u8; MAX_STANDARD_OPCODE_LENGTHS];
        let num_standard = (opcode_base as usize).saturating_sub(1);
        if offset + num_standard > data.len() {
            return Err(DwarfError::Truncated);
        }
        for i in 0..num_standard.min(MAX_STANDARD_OPCODE_LENGTHS) {
            standard_opcode_lengths[i] = data[offset + i];
        }
        offset += num_standard;

        let mut directories = [None; MAX_DIRECTORIES];
        let mut dir_count = 0;
        let mut files = [None; MAX_FILES];
        let mut file_count = 0;

        if version == 4 {
            // DWARF v4: NUL-terminated string sequences
            // Directory table — in DWARF v4, index 0 is the compilation directory
            // (implicit, not stored). The table entries start at index 1.
            offset = parse_v4_directories(data, offset, &mut directories, &mut dir_count)?;
            offset = parse_v4_files(data, offset, &mut files, &mut file_count)?;
        } else {
            // DWARF v5: structured format with content type/form code pairs
            offset = parse_v5_directories(data, offset, &mut directories, &mut dir_count)?;
            offset = parse_v5_files(data, offset, &mut files, &mut file_count)?;
        }

        // The bytecode starts at program_offset regardless of where we ended
        // parsing the header tables — trust header_length.
        let _ = offset;

        Ok(Self {
            version,
            minimum_instruction_length,
            maximum_operations_per_instruction,
            default_is_stmt,
            line_base,
            line_range,
            opcode_base,
            standard_opcode_lengths,
            directories,
            dir_count,
            files,
            file_count,
            program_offset,
            unit_length: total_length,
        })
    }

    /// Returns the file entry at the given index.
    ///
    /// In DWARF v4, file indices are 1-based. In DWARF v5, they are 0-based.
    /// This method handles both conventions transparently — pass the raw
    /// `file_index` from a [`LineRow`](crate::LineRow).
    #[must_use]
    pub fn file(&self, file_index: u64) -> Option<&FileEntry<'a>> {
        let idx = if self.version >= 5 {
            file_index as usize
        } else {
            // DWARF v4: 1-based, so index 1 maps to files[0]
            if file_index == 0 {
                return None;
            }
            (file_index - 1) as usize
        };
        if idx < self.file_count {
            self.files[idx].as_ref()
        } else {
            None
        }
    }

    /// Returns the directory name at the given index.
    #[must_use]
    pub fn directory(&self, dir_index: u64) -> Option<&'a str> {
        let idx = dir_index as usize;
        if idx < self.dir_count {
            self.directories[idx]
        } else {
            None
        }
    }

    /// Returns the number of file entries.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.file_count
    }

    /// Returns the number of directories.
    #[must_use]
    pub fn dir_count(&self) -> usize {
        self.dir_count
    }

    /// Returns an iterator over the line program bytecode rows.
    ///
    /// `unit_data` should be the same data slice passed to [`parse`](Self::parse).
    #[must_use]
    pub fn rows(&self, unit_data: &'a [u8]) -> LineProgramIter<'a> {
        let bytecode_start = self.program_offset;
        let bytecode_end = self.unit_length;
        let bytecode = if bytecode_start < unit_data.len() {
            &unit_data[bytecode_start..bytecode_end.min(unit_data.len())]
        } else {
            &[]
        };
        LineProgramIter::new(self, bytecode)
    }
}

// ---------------------------------------------------------------------------
// DWARF v4 header parsing
// ---------------------------------------------------------------------------

/// Parse DWARF v4 directory table (NUL-terminated strings, terminated by empty string).
fn parse_v4_directories<'a>(
    data: &'a [u8],
    mut offset: usize,
    directories: &mut [Option<&'a str>; MAX_DIRECTORIES],
    dir_count: &mut usize,
) -> Result<usize, DwarfError> {
    loop {
        if offset >= data.len() {
            return Err(DwarfError::Truncated);
        }
        if data[offset] == 0 {
            offset += 1; // consume the terminating zero byte
            break;
        }
        let s = read_nul_str(data, offset)?;
        if *dir_count >= MAX_DIRECTORIES {
            return Err(DwarfError::TooManyDirectories);
        }
        directories[*dir_count] = Some(s);
        *dir_count += 1;
        offset += s.len() + 1; // +1 for NUL
    }
    Ok(offset)
}

/// Parse DWARF v4 file table (entries terminated by a zero byte).
fn parse_v4_files<'a>(
    data: &'a [u8],
    mut offset: usize,
    files: &mut [Option<FileEntry<'a>>; MAX_FILES],
    file_count: &mut usize,
) -> Result<usize, DwarfError> {
    loop {
        if offset >= data.len() {
            return Err(DwarfError::Truncated);
        }
        if data[offset] == 0 {
            offset += 1;
            break;
        }
        let name = read_nul_str(data, offset)?;
        offset += name.len() + 1;

        // directory_index (ULEB128)
        let (dir_idx, consumed) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
        offset += consumed;

        // time (ULEB128) — skip
        let (_, consumed) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
        offset += consumed;

        // size (ULEB128) — skip
        let (_, consumed) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
        offset += consumed;

        if *file_count >= MAX_FILES {
            return Err(DwarfError::TooManyFiles);
        }
        files[*file_count] = Some(FileEntry {
            directory_index: dir_idx,
            name,
        });
        *file_count += 1;
    }
    Ok(offset)
}

// ---------------------------------------------------------------------------
// DWARF v5 header parsing
// ---------------------------------------------------------------------------

/// `DW_LNCT_path` — file path content type code.
const DW_LNCT_PATH: u64 = 0x01;
/// `DW_LNCT_directory_index` — directory index content type code.
const DW_LNCT_DIRECTORY_INDEX: u64 = 0x02;

/// `DW_FORM_string` — inline NUL-terminated string.
const DW_FORM_STRING: u64 = 0x08;
/// `DW_FORM_line_strp` — offset into `.debug_line_str`.
const DW_FORM_LINE_STRP: u64 = 0x1f;
/// `DW_FORM_data1` — 1-byte unsigned integer.
const DW_FORM_DATA1: u64 = 0x0b;
/// `DW_FORM_data2` — 2-byte unsigned integer.
const DW_FORM_DATA2: u64 = 0x05;
/// `DW_FORM_udata` — unsigned LEB128.
const DW_FORM_UDATA: u64 = 0x0f;
/// `DW_FORM_strp` — offset into `.debug_str`.
const DW_FORM_STRP: u64 = 0x0e;

/// Parse DWARF v5 directory table.
fn parse_v5_directories<'a>(
    data: &'a [u8],
    mut offset: usize,
    directories: &mut [Option<&'a str>; MAX_DIRECTORIES],
    dir_count: &mut usize,
) -> Result<usize, DwarfError> {
    // format_count (u8)
    if offset >= data.len() {
        return Err(DwarfError::Truncated);
    }
    let format_count = data[offset] as usize;
    offset += 1;

    // Read format pairs: (content_type: ULEB128, form: ULEB128) * format_count
    let mut formats = [(0u64, 0u64); 8];
    for item in formats.iter_mut().take(format_count.min(8)) {
        let (ct, c1) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
        offset += c1;
        let (form, c2) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
        offset += c2;
        *item = (ct, form);
    }

    // entry_count (ULEB128)
    let (entry_count, consumed) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
    offset += consumed;

    for _ in 0..entry_count {
        let mut path: Option<&'a str> = None;
        for &(ct, form) in formats.iter().take(format_count.min(8)) {
            let (value, new_offset) = read_form_value(data, offset, form)?;
            offset = new_offset;
            if ct == DW_LNCT_PATH {
                path = Some(value.as_str(data)?);
            }
        }
        if *dir_count >= MAX_DIRECTORIES {
            return Err(DwarfError::TooManyDirectories);
        }
        directories[*dir_count] = path;
        *dir_count += 1;
    }

    Ok(offset)
}

/// Parse DWARF v5 file table.
fn parse_v5_files<'a>(
    data: &'a [u8],
    mut offset: usize,
    files: &mut [Option<FileEntry<'a>>; MAX_FILES],
    file_count: &mut usize,
) -> Result<usize, DwarfError> {
    // format_count (u8)
    if offset >= data.len() {
        return Err(DwarfError::Truncated);
    }
    let format_count = data[offset] as usize;
    offset += 1;

    // Read format pairs
    let mut formats = [(0u64, 0u64); 8];
    for item in formats.iter_mut().take(format_count.min(8)) {
        let (ct, c1) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
        offset += c1;
        let (form, c2) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
        offset += c2;
        *item = (ct, form);
    }

    // entry_count (ULEB128)
    let (entry_count, consumed) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
    offset += consumed;

    for _ in 0..entry_count {
        let mut name: Option<&'a str> = None;
        let mut dir_idx: u64 = 0;
        for &(ct, form) in formats.iter().take(format_count.min(8)) {
            let (value, new_offset) = read_form_value(data, offset, form)?;
            offset = new_offset;
            match ct {
                DW_LNCT_PATH => {
                    name = Some(value.as_str(data)?);
                }
                DW_LNCT_DIRECTORY_INDEX => {
                    dir_idx = value.as_u64();
                }
                _ => {} // skip other content types
            }
        }
        if *file_count >= MAX_FILES {
            return Err(DwarfError::TooManyFiles);
        }
        files[*file_count] = Some(FileEntry {
            directory_index: dir_idx,
            name: name.unwrap_or(""),
        });
        *file_count += 1;
    }

    Ok(offset)
}

// ---------------------------------------------------------------------------
// Form value helpers
// ---------------------------------------------------------------------------

/// A decoded DWARF form value (used internally during header parsing).
#[derive(Clone, Copy)]
enum FormValue {
    /// A NUL-terminated string at this offset in the data.
    StringOffset(usize, usize), // (offset_into_data, len)
    /// An unsigned integer.
    Uint(u64),
    /// An offset into an external string section (`.debug_str` or `.debug_line_str`).
    /// We can't resolve these without the external section, so treat as empty.
    #[allow(dead_code, reason = "reserved for Phase 7 external string resolution")]
    ExternalStrOffset(u64),
}

impl FormValue {
    /// Extracts a `&str` from the form value.
    fn as_str<'a>(self, data: &'a [u8]) -> Result<&'a str, DwarfError> {
        match self {
            Self::StringOffset(off, len) => {
                if off + len > data.len() {
                    return Err(DwarfError::Truncated);
                }
                core::str::from_utf8(&data[off..off + len]).map_err(|_| DwarfError::InvalidUtf8)
            }
            Self::Uint(_) => Ok(""),
            Self::ExternalStrOffset(_) => Ok("<external>"),
        }
    }

    /// Extracts a `u64` from the form value.
    fn as_u64(self) -> u64 {
        match self {
            Self::Uint(v) => v,
            _ => 0,
        }
    }
}

/// Read a DWARF form value from data at the given offset.
fn read_form_value(
    data: &[u8],
    offset: usize,
    form: u64,
) -> Result<(FormValue, usize), DwarfError> {
    match form {
        DW_FORM_STRING => {
            let s = read_nul_str(data, offset)?;
            let len = s.len();
            Ok((
                FormValue::StringOffset(offset, len),
                offset + len + 1, // +1 for NUL
            ))
        }
        DW_FORM_LINE_STRP | DW_FORM_STRP => {
            // 4-byte offset into .debug_line_str or .debug_str
            if offset + 4 > data.len() {
                return Err(DwarfError::Truncated);
            }
            let str_offset = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            Ok((
                FormValue::ExternalStrOffset(u64::from(str_offset)),
                offset + 4,
            ))
        }
        DW_FORM_DATA1 => {
            if offset >= data.len() {
                return Err(DwarfError::Truncated);
            }
            Ok((FormValue::Uint(u64::from(data[offset])), offset + 1))
        }
        DW_FORM_DATA2 => {
            if offset + 2 > data.len() {
                return Err(DwarfError::Truncated);
            }
            let v = u16::from_le_bytes([data[offset], data[offset + 1]]);
            Ok((FormValue::Uint(u64::from(v)), offset + 2))
        }
        DW_FORM_UDATA => {
            let (v, consumed) = decode_uleb128(&data[offset..]).ok_or(DwarfError::Truncated)?;
            Ok((FormValue::Uint(v), offset + consumed))
        }
        _ => {
            // Unknown form — we can't know the size, so this is an error
            Err(DwarfError::InvalidOffset)
        }
    }
}

/// Read a NUL-terminated UTF-8 string from data at the given offset.
fn read_nul_str(data: &[u8], offset: usize) -> Result<&str, DwarfError> {
    if offset >= data.len() {
        return Err(DwarfError::Truncated);
    }
    let remaining = &data[offset..];
    let nul_pos = remaining
        .iter()
        .position(|&b| b == 0)
        .ok_or(DwarfError::Truncated)?;
    core::str::from_utf8(&remaining[..nul_pos]).map_err(|_| DwarfError::InvalidUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal DWARF v4 line program header.
    fn make_v4_line_program() -> Vec<u8> {
        let mut buf = Vec::new();

        // We'll fill in unit_length at the end
        buf.extend_from_slice(&[0u8; 4]); // placeholder for unit_length

        // Version: 4
        buf.extend_from_slice(&4u16.to_le_bytes());

        // header_length: placeholder (filled below)
        let header_length_pos = buf.len();
        buf.extend_from_slice(&[0u8; 4]);

        let header_start = buf.len();

        // minimum_instruction_length: 1
        buf.push(1);
        // maximum_operations_per_instruction: 1
        buf.push(1);
        // default_is_stmt: 1
        buf.push(1);
        // line_base: -5 (0xFB as i8)
        buf.push((-5i8) as u8);
        // line_range: 14
        buf.push(14);
        // opcode_base: 13
        buf.push(13);
        // standard_opcode_lengths (12 entries for opcodes 1-12)
        buf.extend_from_slice(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]);

        // Directory table: one dir "src", then terminator
        buf.extend_from_slice(b"src\0");
        buf.push(0); // end of directories

        // File table: one file "main.rs" in dir 1, then terminator
        buf.extend_from_slice(b"main.rs\0");
        buf.push(1); // directory index
        buf.push(0); // time
        buf.push(0); // size
        buf.push(0); // end of files

        let header_end = buf.len();

        // Fix header_length
        let header_length = (header_end - header_start) as u32;
        buf[header_length_pos..header_length_pos + 4].copy_from_slice(&header_length.to_le_bytes());

        // Add a minimal line program (just DW_LNE_end_sequence)
        // Extended opcode: 0x00, length=1, opcode=1 (end_sequence)
        buf.push(0x00); // extended opcode marker
        buf.push(0x01); // length of extended opcode (1 byte)
        buf.push(0x01); // DW_LNE_end_sequence

        // Fix unit_length
        let unit_length = (buf.len() - 4) as u32;
        buf[0..4].copy_from_slice(&unit_length.to_le_bytes());

        buf
    }

    #[test]
    fn parse_v4_header() {
        let buf = make_v4_line_program();
        let header = LineProgramHeader::parse(&buf).expect("valid v4 header");

        assert_eq!(header.version, 4);
        assert_eq!(header.minimum_instruction_length, 1);
        assert_eq!(header.maximum_operations_per_instruction, 1);
        assert!(header.default_is_stmt);
        assert_eq!(header.line_base, -5);
        assert_eq!(header.line_range, 14);
        assert_eq!(header.opcode_base, 13);
    }

    #[test]
    fn v4_directory_table() {
        let buf = make_v4_line_program();
        let header = LineProgramHeader::parse(&buf).expect("valid v4 header");

        assert_eq!(header.dir_count(), 1);
        assert_eq!(header.directory(0), Some("src"));
        assert_eq!(header.directory(1), None);
    }

    #[test]
    fn v4_file_table() {
        let buf = make_v4_line_program();
        let header = LineProgramHeader::parse(&buf).expect("valid v4 header");

        assert_eq!(header.file_count(), 1);
        // DWARF v4 files are 1-indexed
        let file = header.file(1).expect("file 1 exists");
        assert_eq!(file.name, "main.rs");
        assert_eq!(file.directory_index, 1);
        assert!(header.file(0).is_none());
        assert!(header.file(2).is_none());
    }

    #[test]
    fn reject_unsupported_version() {
        let mut buf = make_v4_line_program();
        // Change version to 3
        buf[4..6].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(
            LineProgramHeader::parse(&buf).unwrap_err(),
            DwarfError::UnsupportedVersion
        );
    }

    #[test]
    fn reject_truncated() {
        assert_eq!(
            LineProgramHeader::parse(&[]).unwrap_err(),
            DwarfError::Truncated
        );
        assert_eq!(
            LineProgramHeader::parse(&[0, 0]).unwrap_err(),
            DwarfError::Truncated
        );
    }
}
