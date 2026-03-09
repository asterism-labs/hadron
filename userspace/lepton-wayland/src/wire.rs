//! Wayland wire protocol encoder/decoder.
//!
//! All values are little-endian. Message layout:
//!   [object_id: u32][size_and_opcode: u32][args...]
//! where `size_and_opcode = (total_size << 16) | opcode` and `total_size`
//! includes the 8-byte header.
//!
//! Argument types:
//!   uint / int  — 4 bytes LE
//!   fd          — zero inline bytes; carried OOB as SCM_RIGHTS
//!   string      — u32 length (incl. null), then bytes+null, padded to 4-byte
//!   array       — u32 byte-length, then bytes, padded to 4-byte
//!   new_id<*>   — string (interface name) + u32 version + u32 id

extern crate alloc;
use alloc::vec::Vec;

// -- Encoding ----------------------------------------------------------------

/// Append a `u32` (little-endian) to `buf`.
#[inline]
pub fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Append an `i32` (little-endian) to `buf`.
#[inline]
pub fn push_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Append a Wayland-encoded string (length-prefixed, null-terminated, 4-byte padded).
pub fn push_str(buf: &mut Vec<u8>, s: &[u8]) {
    // length field includes the null terminator
    let len = s.len() + 1;
    push_u32(buf, len as u32);
    buf.extend_from_slice(s);
    buf.push(0); // null terminator
    // pad so that (4 + len) is a multiple of 4
    let pad = (4 - ((4 + len) & 3)) & 3;
    for _ in 0..pad {
        buf.push(0);
    }
}

/// Append a Wayland-encoded byte array (length-prefixed, 4-byte padded).
pub fn push_array(buf: &mut Vec<u8>, data: &[u8]) {
    push_u32(buf, data.len() as u32);
    buf.extend_from_slice(data);
    let pad = (4 - (data.len() & 3)) & 3;
    for _ in 0..pad {
        buf.push(0);
    }
}

/// Begin a Wayland message for `(obj, opcode)`.
///
/// Returns the byte offset of the message start so [`end_msg`] can patch
/// the size field. The caller appends argument bytes after this call.
pub fn begin_msg(buf: &mut Vec<u8>, obj_id: u32, opcode: u16) -> usize {
    let start = buf.len();
    push_u32(buf, obj_id);
    push_u32(buf, 0); // size+opcode placeholder
    let _ = opcode; // stored in end_msg
    start
}

/// Finish the message started at `start` by patching the size+opcode field.
pub fn end_msg(buf: &mut Vec<u8>, start: usize, opcode: u16) {
    let total = (buf.len() - start) as u32;
    let size_op = (total << 16) | (opcode as u32);
    let bytes = size_op.to_le_bytes();
    buf[start + 4] = bytes[0];
    buf[start + 5] = bytes[1];
    buf[start + 6] = bytes[2];
    buf[start + 7] = bytes[3];
}

// -- Decoding ----------------------------------------------------------------

/// Parse a Wayland message header from the front of `data`.
///
/// Returns `(object_id, opcode, total_size_bytes)` or `None` if `data` is
/// too short to hold a complete message.
pub fn parse_header(data: &[u8]) -> Option<(u32, u16, usize)> {
    if data.len() < 8 {
        return None;
    }
    let obj = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let size_op = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let size = (size_op >> 16) as usize;
    let opcode = (size_op & 0xFFFF) as u16;
    if size < 8 || data.len() < size {
        return None;
    }
    Some((obj, opcode, size))
}

/// Read a `u32` from `args[offset..]`. Returns `(value, new_offset)`.
pub fn read_u32(args: &[u8], offset: usize) -> Option<(u32, usize)> {
    let end = offset.checked_add(4)?;
    if args.len() < end {
        return None;
    }
    let v = u32::from_le_bytes([
        args[offset],
        args[offset + 1],
        args[offset + 2],
        args[offset + 3],
    ]);
    Some((v, end))
}

/// Read an `i32` from `args[offset..]`. Returns `(value, new_offset)`.
pub fn read_i32(args: &[u8], offset: usize) -> Option<(i32, usize)> {
    let (v, o) = read_u32(args, offset)?;
    Some((v as i32, o))
}

/// Read a Wayland string from `args[offset..]`.
///
/// Returns `(bytes_without_null, new_offset)`. An empty string (len=0) returns
/// an empty slice.
pub fn read_str<'a>(args: &'a [u8], offset: usize) -> Option<(&'a [u8], usize)> {
    let (len_u32, data_start) = read_u32(args, offset)?;
    let len = len_u32 as usize;
    if len == 0 {
        return Some((&[], data_start));
    }
    let data_end = data_start.checked_add(len)?;
    if args.len() < data_end {
        return None;
    }
    // String bytes excluding the null terminator
    let s = &args[data_start..data_end - 1];
    // Total bytes consumed from `offset`: 4 (length field) + len, rounded up to 4
    let raw = 4usize + len;
    let aligned = (raw + 3) & !3;
    Some((s, offset + aligned))
}

/// Read a Wayland array from `args[offset..]`.
///
/// Returns `(byte_slice, new_offset)`.
pub fn read_array<'a>(args: &'a [u8], offset: usize) -> Option<(&'a [u8], usize)> {
    let (len_u32, data_start) = read_u32(args, offset)?;
    let len = len_u32 as usize;
    let data_end = data_start.checked_add(len)?;
    if args.len() < data_end {
        return None;
    }
    let data = &args[data_start..data_end];
    let raw = 4usize + len;
    let aligned = (raw + 3) & !3;
    Some((data, offset + aligned))
}
