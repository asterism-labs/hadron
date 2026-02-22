//! AML bytecode parser.
//!
//! Provides [`walk_aml`], a single-pass walker that dispatches to an
//! [`AmlVisitor`] as it encounters namespace objects (devices, scopes,
//! methods, name objects). The parser does not evaluate control flow or
//! method bodies — it extracts the static namespace topology.

use hadron_binparse::BinaryReader;

use super::path::{AmlPath, NameSeg};
use super::value::{AmlError, AmlValue, EisaId, InlineString};
use super::visitor::AmlVisitor;

/// Walk AML bytecode and dispatch namespace objects to the visitor.
///
/// `data` should be the raw AML bytecode from a DSDT or SSDT table
/// (everything after the SDT header).
///
/// # Errors
///
/// Returns [`AmlError`] if the bytecode is malformed or truncated.
pub fn walk_aml(data: &[u8], visitor: &mut impl AmlVisitor) -> Result<(), AmlError> {
    let mut path = AmlPath::new();
    parse_term_list(data, data.len(), &mut path, visitor)
}

/// Parse a TermList (sequence of TermObj) up to `end` bytes from `data` start.
fn parse_term_list(
    data: &[u8],
    end: usize,
    path: &mut AmlPath,
    visitor: &mut impl AmlVisitor,
) -> Result<(), AmlError> {
    let mut reader = BinaryReader::new(data);

    while reader.position() < end && reader.position() < reader.len() {
        if parse_term_obj(&mut reader, end, path, visitor).is_err() {
            // On error, skip to end of this scope rather than aborting
            // the entire walk. This allows partial parsing of valid regions.
            break;
        }
    }

    Ok(())
}

/// Parse a single TermObj and dispatch to the visitor.
fn parse_term_obj(
    reader: &mut BinaryReader<'_>,
    scope_end: usize,
    path: &mut AmlPath,
    visitor: &mut impl AmlVisitor,
) -> Result<(), AmlError> {
    if reader.position() >= scope_end || reader.is_at_end() {
        return Err(AmlError::UnexpectedEnd);
    }

    let op = reader.read::<u8>().ok_or(AmlError::UnexpectedEnd)?;

    match op {
        // DefName — NameString DataRefObject
        0x08 => parse_def_name(reader, path, visitor),

        // DefScope — PkgLength NameString TermList
        0x10 => parse_def_scope(reader, path, visitor),

        // DefMethod — PkgLength NameString MethodFlags TermList
        0x14 => parse_def_method(reader, path, visitor),

        // ExtOpPrefix
        0x5B => {
            let ext_op = reader.read::<u8>().ok_or(AmlError::UnexpectedEnd)?;
            match ext_op {
                // DefOpRegion — skip
                0x80 => skip_op_region(reader),
                // DefField — skip via PkgLength
                0x81 => skip_pkg_length_block(reader),
                // DefDevice
                0x82 => parse_def_device(reader, path, visitor),
                // DefProcessor
                0x83 => parse_def_processor(reader, path, visitor),
                // DefPowerRes
                0x84 => parse_def_power_res(reader, path, visitor),
                // DefThermalZone
                0x85 => parse_def_thermal_zone(reader, path, visitor),
                // DefMutex, DefEvent — skip name + flags
                0x01 | 0x02 => {
                    skip_name_string(reader)?;
                    reader.skip(1);
                    Ok(())
                }
                // DefIndexField, DefBankField — skip via PkgLength
                0x86 | 0x87 => skip_pkg_length_block(reader),
                // DefCreateField variants — skip
                0x13 | 0x0D => {
                    // Skip to end: these have complex operands. Best effort.
                    Err(AmlError::InvalidAml)
                }
                // Unknown extended opcode — try to skip via PkgLength
                _ => skip_pkg_length_block(reader),
            }
        }

        // Data object prefixes — skip their payloads
        // ByteConst
        0x0A => {
            reader.skip(1);
            Ok(())
        }
        // WordConst
        0x0B => {
            reader.skip(2);
            Ok(())
        }
        // DWordConst
        0x0C => {
            reader.skip(4);
            Ok(())
        }
        // StringConst
        0x0D => {
            skip_string(reader);
            Ok(())
        }
        // QWordConst
        0x0E => {
            reader.skip(8);
            Ok(())
        }

        // DefBuffer — PkgLength ...
        0x11 => skip_pkg_length_block(reader),
        // DefPackage / DefVarPackage
        0x12 | 0x13 => skip_pkg_length_block(reader),

        // Zero, One, Ones — no operands
        0x00 | 0x01 | 0xFF => Ok(()),

        // Local0-Local7, Arg0-Arg6 — no operands
        0x60..=0x67 | 0x68..=0x6E => Ok(()),

        // DefStore, DefAdd, etc. — complex operands, skip as unknown
        // NameString (lead name char A-Z or _)
        b'A'..=b'Z' | b'_' | b'\\' | b'^' | b'.' | b'/' => {
            // This might be a name reference. Unread the byte and skip the name.
            // We can't easily unread, so just skip remaining name chars.
            skip_remaining_name_after_lead(reader, op)?;
            Ok(())
        }

        // DefIf, DefElse, DefWhile — PkgLength blocks
        0xA0 | 0xA1 | 0xA2 => skip_pkg_length_block(reader),

        // DefReturn — skip the return value object
        0xA4 => {
            // Return has one TermArg — try to skip it
            skip_data_object(reader)
        }

        // NoOp
        0xA3 => Ok(()),

        // DefBreak
        0xA5 => Ok(()),

        // Other — skip one byte (best effort)
        _ => Ok(()),
    }
}

/// Parse DefName: 0x08 NameString DataRefObject
fn parse_def_name(
    reader: &mut BinaryReader<'_>,
    path: &AmlPath,
    visitor: &mut impl AmlVisitor,
) -> Result<(), AmlError> {
    let name = read_name_seg(reader)?;
    let value = resolve_data_object(reader);
    visitor.name_object(path, name, &value);
    Ok(())
}

/// Parse DefScope: 0x10 PkgLength NameString TermList
fn parse_def_scope(
    reader: &mut BinaryReader<'_>,
    path: &mut AmlPath,
    visitor: &mut impl AmlVisitor,
) -> Result<(), AmlError> {
    let (pkg_end, _pkg_len) = decode_pkg_length(reader)?;
    let abs_end = reader.position() + pkg_end;

    let name = read_name_path(reader, path)?;

    path.push(name);
    visitor.enter_scope(path);

    let remaining = reader.remaining();
    let block_len = abs_end
        .saturating_sub(reader.position())
        .min(remaining.len());
    parse_term_list(&remaining[..block_len], block_len, path, visitor)?;

    // Advance reader past the scope body
    let skip_to = abs_end.min(reader.len());
    let skip_amount = skip_to.saturating_sub(reader.position());
    reader.skip(skip_amount);

    visitor.exit_scope();
    path.pop();

    Ok(())
}

/// Parse DefMethod: 0x14 PkgLength NameString MethodFlags TermList
fn parse_def_method(
    reader: &mut BinaryReader<'_>,
    path: &AmlPath,
    visitor: &mut impl AmlVisitor,
) -> Result<(), AmlError> {
    let (pkg_end, _pkg_len) = decode_pkg_length(reader)?;
    let abs_end = reader.position() + pkg_end;

    let name = read_name_seg(reader)?;
    let method_flags = reader.read::<u8>().ok_or(AmlError::UnexpectedEnd)?;
    let arg_count = method_flags & 0x07;
    let serialized = (method_flags & 0x08) != 0;

    visitor.method(path, name, arg_count, serialized);

    // Skip method body — we don't evaluate it.
    let skip_to = abs_end.min(reader.len());
    let skip_amount = skip_to.saturating_sub(reader.position());
    reader.skip(skip_amount);

    Ok(())
}

/// Parse DefDevice: 0x5B 0x82 PkgLength NameString TermList
fn parse_def_device(
    reader: &mut BinaryReader<'_>,
    path: &mut AmlPath,
    visitor: &mut impl AmlVisitor,
) -> Result<(), AmlError> {
    let (pkg_end, _pkg_len) = decode_pkg_length(reader)?;
    let abs_end = reader.position() + pkg_end;

    let name = read_name_seg(reader)?;

    path.push(name);
    visitor.device(path, name);
    visitor.enter_scope(path);

    let remaining = reader.remaining();
    let block_len = abs_end
        .saturating_sub(reader.position())
        .min(remaining.len());
    parse_term_list(&remaining[..block_len], block_len, path, visitor)?;

    let skip_to = abs_end.min(reader.len());
    let skip_amount = skip_to.saturating_sub(reader.position());
    reader.skip(skip_amount);

    visitor.exit_scope();
    path.pop();

    Ok(())
}

/// Parse DefProcessor: 0x5B 0x83 PkgLength NameString ProcID PblkAddr PblkLen
fn parse_def_processor(
    reader: &mut BinaryReader<'_>,
    path: &AmlPath,
    visitor: &mut impl AmlVisitor,
) -> Result<(), AmlError> {
    let (pkg_end, _pkg_len) = decode_pkg_length(reader)?;
    let abs_end = reader.position() + pkg_end;

    let name = read_name_seg(reader)?;
    let proc_id = reader.read::<u8>().ok_or(AmlError::UnexpectedEnd)?;

    visitor.processor(path, name, proc_id);

    // Skip rest of processor block.
    let skip_to = abs_end.min(reader.len());
    let skip_amount = skip_to.saturating_sub(reader.position());
    reader.skip(skip_amount);

    Ok(())
}

/// Parse DefPowerRes: 0x5B 0x84 PkgLength NameString SystemLevel ResourceOrder
fn parse_def_power_res(
    reader: &mut BinaryReader<'_>,
    path: &AmlPath,
    visitor: &mut impl AmlVisitor,
) -> Result<(), AmlError> {
    let (pkg_end, _pkg_len) = decode_pkg_length(reader)?;
    let abs_end = reader.position() + pkg_end;

    let name = read_name_seg(reader)?;

    visitor.power_resource(path, name);

    let skip_to = abs_end.min(reader.len());
    let skip_amount = skip_to.saturating_sub(reader.position());
    reader.skip(skip_amount);

    Ok(())
}

/// Parse DefThermalZone: 0x5B 0x85 PkgLength NameString TermList
fn parse_def_thermal_zone(
    reader: &mut BinaryReader<'_>,
    path: &mut AmlPath,
    visitor: &mut impl AmlVisitor,
) -> Result<(), AmlError> {
    let (pkg_end, _pkg_len) = decode_pkg_length(reader)?;
    let abs_end = reader.position() + pkg_end;

    let name = read_name_seg(reader)?;

    path.push(name);
    visitor.thermal_zone(path, name);
    visitor.enter_scope(path);

    let remaining = reader.remaining();
    let block_len = abs_end
        .saturating_sub(reader.position())
        .min(remaining.len());
    parse_term_list(&remaining[..block_len], block_len, path, visitor)?;

    let skip_to = abs_end.min(reader.len());
    let skip_amount = skip_to.saturating_sub(reader.position());
    reader.skip(skip_amount);

    visitor.exit_scope();
    path.pop();

    Ok(())
}

/// Skip an OpRegion: NameString RegionSpace RegionOffset RegionLen
fn skip_op_region(reader: &mut BinaryReader<'_>) -> Result<(), AmlError> {
    skip_name_string(reader)?;
    // RegionSpace (1 byte)
    reader.skip(1);
    // RegionOffset — TermArg (integer)
    skip_data_object(reader)?;
    // RegionLen — TermArg (integer)
    skip_data_object(reader)
}

/// Skip a PkgLength-delimited block.
fn skip_pkg_length_block(reader: &mut BinaryReader<'_>) -> Result<(), AmlError> {
    let (pkg_remaining, _) = decode_pkg_length(reader)?;
    let skip_to = (reader.position() + pkg_remaining).min(reader.len());
    let skip_amount = skip_to.saturating_sub(reader.position());
    reader.skip(skip_amount);
    Ok(())
}

// ─── Name parsing ───────────────────────────────────────────────────────────

/// Read a single 4-byte NameSeg from the reader.
fn read_name_seg(reader: &mut BinaryReader<'_>) -> Result<NameSeg, AmlError> {
    let bytes: [u8; 4] = reader.read().ok_or(AmlError::UnexpectedEnd)?;
    Ok(NameSeg(bytes))
}

/// Read an AML name path (handling prefix chars) and return the final NameSeg.
///
/// For complex multi-segment paths, this returns the last segment which
/// is the local name within the current scope.
fn read_name_path(reader: &mut BinaryReader<'_>, _path: &AmlPath) -> Result<NameSeg, AmlError> {
    // Skip prefix chars: '\' (root), '^' (parent)
    loop {
        let remaining = reader.remaining();
        if remaining.is_empty() {
            return Err(AmlError::UnexpectedEnd);
        }
        match remaining[0] {
            b'\\' | b'^' => {
                reader.skip(1);
            }
            _ => break,
        }
    }

    let remaining = reader.remaining();
    if remaining.is_empty() {
        return Err(AmlError::UnexpectedEnd);
    }

    match remaining[0] {
        // NullName
        0x00 => {
            reader.skip(1);
            Ok(NameSeg(*b"____"))
        }
        // DualNamePath
        0x2E => {
            reader.skip(1);
            let _first = read_name_seg(reader)?;
            let second = read_name_seg(reader)?;
            Ok(second)
        }
        // MultiNamePath
        0x2F => {
            reader.skip(1);
            let seg_count = reader.read::<u8>().ok_or(AmlError::UnexpectedEnd)?;
            let mut last = NameSeg(*b"____");
            for _ in 0..seg_count {
                last = read_name_seg(reader)?;
            }
            Ok(last)
        }
        // Regular NameSeg (lead name char)
        b'A'..=b'Z' | b'_' => read_name_seg(reader),
        _ => Err(AmlError::InvalidAml),
    }
}

/// Skip a NameString without extracting it.
fn skip_name_string(reader: &mut BinaryReader<'_>) -> Result<(), AmlError> {
    // Skip prefix chars
    loop {
        let remaining = reader.remaining();
        if remaining.is_empty() {
            return Err(AmlError::UnexpectedEnd);
        }
        match remaining[0] {
            b'\\' | b'^' => {
                reader.skip(1);
            }
            _ => break,
        }
    }

    let remaining = reader.remaining();
    if remaining.is_empty() {
        return Err(AmlError::UnexpectedEnd);
    }

    match remaining[0] {
        0x00 => {
            reader.skip(1);
            Ok(())
        }
        0x2E => {
            reader.skip(1 + 8); // DualNamePath: prefix + 2 NameSegs
            Ok(())
        }
        0x2F => {
            reader.skip(1);
            let seg_count = reader.read::<u8>().ok_or(AmlError::UnexpectedEnd)? as usize;
            reader.skip(seg_count * 4);
            Ok(())
        }
        b'A'..=b'Z' | b'_' => {
            reader.skip(4);
            Ok(())
        }
        _ => Err(AmlError::InvalidAml),
    }
}

/// Skip remaining name characters after the first byte has already been read.
fn skip_remaining_name_after_lead(reader: &mut BinaryReader<'_>, lead: u8) -> Result<(), AmlError> {
    match lead {
        b'\\' | b'^' => skip_name_string(reader),
        b'.' => {
            // DualNamePath already consumed prefix byte
            reader.skip(8);
            Ok(())
        }
        b'/' => {
            // MultiNamePath
            let seg_count = reader.read::<u8>().ok_or(AmlError::UnexpectedEnd)? as usize;
            reader.skip(seg_count * 4);
            Ok(())
        }
        b'A'..=b'Z' | b'_' => {
            // Single NameSeg, 3 more bytes
            reader.skip(3);
            Ok(())
        }
        _ => Ok(()),
    }
}

// ─── PkgLength decoding ────────────────────────────────────────────────────

/// Decode an ACPI PkgLength field (1-4 bytes).
///
/// Returns `(remaining_bytes, total_pkg_length)` where `remaining_bytes` is
/// the number of bytes after the PkgLength field itself that belong to this
/// package.
fn decode_pkg_length(reader: &mut BinaryReader<'_>) -> Result<(usize, usize), AmlError> {
    let lead = reader.read::<u8>().ok_or(AmlError::UnexpectedEnd)?;
    let byte_count = (lead >> 6) & 0x03;

    if byte_count == 0 {
        // Single byte: bits 5:0 are the length
        let len = (lead & 0x3F) as usize;
        // PkgLength includes itself (1 byte), so remaining = len - 1
        let remaining = len.saturating_sub(1);
        return Ok((remaining, len));
    }

    // Multi-byte: lead bits 3:0 are low nibble, following bytes are higher bits
    let mut length = (lead & 0x0F) as usize;

    for i in 0..byte_count {
        let b = reader.read::<u8>().ok_or(AmlError::UnexpectedEnd)?;
        length |= (b as usize) << (4 + i * 8);
    }

    // PkgLength includes itself (1 + byte_count bytes)
    let header_size = 1 + byte_count as usize;
    let remaining = length.saturating_sub(header_size);
    Ok((remaining, length))
}

// ─── Data object resolution ────────────────────────────────────────────────

/// Try to resolve a DataRefObject (the value part of a DefName).
fn resolve_data_object(reader: &mut BinaryReader<'_>) -> AmlValue {
    let remaining = reader.remaining();
    if remaining.is_empty() {
        return AmlValue::Unresolved;
    }

    match remaining[0] {
        // ZeroOp
        0x00 => {
            reader.skip(1);
            AmlValue::Integer(0)
        }
        // OneOp
        0x01 => {
            reader.skip(1);
            AmlValue::Integer(1)
        }
        // OnesOp
        0xFF => {
            reader.skip(1);
            AmlValue::Integer(u64::MAX)
        }
        // ByteConst
        0x0A => {
            reader.skip(1);
            match reader.read::<u8>() {
                Some(v) => AmlValue::Integer(u64::from(v)),
                None => AmlValue::Unresolved,
            }
        }
        // WordConst
        0x0B => {
            reader.skip(1);
            match reader.read::<u16>() {
                Some(v) => AmlValue::Integer(u64::from(v)),
                None => AmlValue::Unresolved,
            }
        }
        // DWordConst
        0x0C => {
            reader.skip(1);
            match reader.read::<u32>() {
                Some(v) => AmlValue::Integer(u64::from(v)),
                None => AmlValue::Unresolved,
            }
        }
        // StringConst
        0x0D => {
            reader.skip(1);
            let start = reader.position();
            skip_string(reader);
            let end = reader.position().saturating_sub(1); // exclude null terminator
            let data = reader.data();
            let str_bytes = data.get(start..end).unwrap_or(&[]);
            AmlValue::String(InlineString::from_bytes(str_bytes))
        }
        // QWordConst
        0x0E => {
            reader.skip(1);
            match reader.read::<u64>() {
                Some(v) => AmlValue::Integer(v),
                None => AmlValue::Unresolved,
            }
        }
        // Buffer — check for EISAID pattern
        0x11 => try_resolve_eisaid(reader),
        // Package / VarPackage
        0x12 | 0x13 => {
            reader.skip(1);
            let _ = skip_pkg_length_block_inner(reader);
            AmlValue::Unresolved
        }
        // Revision op
        0x5B => {
            if remaining.get(1) == Some(&0x30) {
                reader.skip(2);
                // RevisionOp — the ACPI revision, treat as integer
                AmlValue::Integer(2)
            } else {
                AmlValue::Unresolved
            }
        }
        _ => AmlValue::Unresolved,
    }
}

/// Try to resolve an EisaId from a Buffer data object.
///
/// EISAID encoding: `Buffer(4) { DWordConst }` where the DWord is the
/// compressed EISA ID. In AML bytecode this appears as:
/// `0x11 PkgLen 0x0C DWordData`
fn try_resolve_eisaid(reader: &mut BinaryReader<'_>) -> AmlValue {
    let pos = reader.position();
    reader.skip(1); // skip 0x11 (Buffer op)

    // Decode the PkgLength
    let pkg_result = decode_pkg_length(reader);
    let Ok((pkg_remaining, _)) = pkg_result else {
        // Rewind isn't possible with BinaryReader, return Unresolved
        return AmlValue::Unresolved;
    };

    let body_end = reader.position() + pkg_remaining;

    // Buffer size — should be a small integer (typically ByteConst 0x04)
    let remaining = reader.remaining();
    if remaining.is_empty() {
        skip_to(reader, body_end);
        return AmlValue::Unresolved;
    }

    let buf_size = match remaining[0] {
        0x0A => {
            reader.skip(1);
            reader.read::<u8>().map(u32::from)
        }
        0x0C => {
            reader.skip(1);
            reader.read::<u32>()
        }
        0x00 => {
            reader.skip(1);
            Some(0)
        }
        _ => None,
    };

    if buf_size == Some(4) {
        // Read the initializer — should be a DWordConst
        let remaining = reader.remaining();
        if remaining.first() == Some(&0x0C) {
            reader.skip(1);
            if let Some(raw) = reader.read::<u32>() {
                skip_to(reader, body_end);
                return AmlValue::EisaId(EisaId { raw });
            }
        } else if remaining.len() >= 4 {
            // Might be raw byte data
            if let Some(raw) = reader.read::<u32>() {
                skip_to(reader, body_end);
                return AmlValue::EisaId(EisaId { raw });
            }
        }
    }

    // Not an EISAID pattern — skip the rest of the buffer
    skip_to(reader, body_end);
    let _ = pos; // suppress unused warning
    AmlValue::Unresolved
}

/// Skip a data object (used when we need to skip TermArgs like in Return).
fn skip_data_object(reader: &mut BinaryReader<'_>) -> Result<(), AmlError> {
    let remaining = reader.remaining();
    if remaining.is_empty() {
        return Err(AmlError::UnexpectedEnd);
    }

    match remaining[0] {
        0x00 | 0x01 | 0xFF => {
            reader.skip(1);
            Ok(())
        }
        0x0A => {
            reader.skip(2);
            Ok(())
        }
        0x0B => {
            reader.skip(3);
            Ok(())
        }
        0x0C => {
            reader.skip(5);
            Ok(())
        }
        0x0D => {
            reader.skip(1);
            skip_string(reader);
            Ok(())
        }
        0x0E => {
            reader.skip(9);
            Ok(())
        }
        0x11 | 0x12 | 0x13 => {
            reader.skip(1);
            skip_pkg_length_block_inner(reader)
        }
        0x60..=0x6E => {
            reader.skip(1);
            Ok(())
        }
        b'A'..=b'Z' | b'_' | b'\\' | b'^' => skip_name_string(reader),
        _ => {
            reader.skip(1);
            Ok(())
        }
    }
}

// ─── Helper functions ──────────────────────────────────────────────────────

/// Skip a null-terminated string.
fn skip_string(reader: &mut BinaryReader<'_>) {
    while let Some(b) = reader.read::<u8>() {
        if b == 0 {
            return;
        }
    }
}

/// Skip a PkgLength block body (PkgLength has not been consumed yet).
fn skip_pkg_length_block_inner(reader: &mut BinaryReader<'_>) -> Result<(), AmlError> {
    let (pkg_remaining, _) = decode_pkg_length(reader)?;
    let target = (reader.position() + pkg_remaining).min(reader.len());
    let amount = target.saturating_sub(reader.position());
    reader.skip(amount);
    Ok(())
}

/// Advance reader to target position.
fn skip_to(reader: &mut BinaryReader<'_>, target: usize) {
    let target = target.min(reader.len());
    let amount = target.saturating_sub(reader.position());
    reader.skip(amount);
}
