//! CPIO initrd archive and embedded binary lookup.
//!
//! The build system produces a single `initrd.cpio` archive (newc format)
//! containing all userspace binaries. This module embeds the archive at
//! compile time and provides lookup functions that parse CPIO headers to
//! locate entries by name.

/// Raw bytes of the initrd CPIO archive.
static INITRD_CPIO: &[u8] = include_bytes!("../../../build/initrd.cpio");

/// CPIO newc header size in bytes.
const HEADER_SIZE: usize = 110;
/// CPIO newc magic bytes.
const MAGIC: &[u8; 6] = b"070701";
/// CPIO trailer sentinel name.
const TRAILER: &[u8] = b"TRAILER!!!";

/// Returns the embedded userboot ELF binary from the initrd.
///
/// # Panics
///
/// Panics if no `bin/userboot` entry is found in the initrd archive.
pub fn elf_bytes() -> &'static [u8] {
    lookup_initrd_binary("userboot").expect("userboot not found in initrd")
}

/// Look up an embedded binary by name in the CPIO initrd.
///
/// Searches for entries matching `bin/<name>` or `<name>` directly.
/// Returns a zero-copy slice into the static CPIO data.
pub fn lookup_initrd_binary(path: &str) -> Option<&'static [u8]> {
    // Strip leading path components to get the binary name.
    let name = path.rsplit('/').next().unwrap_or(path);

    let mut offset = 0usize;
    while offset + HEADER_SIZE <= INITRD_CPIO.len() {
        // Validate magic.
        if &INITRD_CPIO[offset..offset + 6] != MAGIC {
            return None;
        }

        // Parse namesize and filesize from the ASCII hex header fields.
        let namesize = parse_hex_field(offset + 94, 8)? as usize;
        let filesize = parse_hex_field(offset + 54, 8)? as usize;

        // Name starts right after the 110-byte header.
        let name_start = offset + HEADER_SIZE;
        let name_end = name_start + namesize.checked_sub(1)?; // exclude NUL

        if name_end > INITRD_CPIO.len() {
            return None;
        }

        let entry_name = &INITRD_CPIO[name_start..name_end];

        // Check for trailer.
        if entry_name == TRAILER {
            return None;
        }

        // Data starts after header + namesize, aligned to 4 bytes.
        let data_offset = align4(offset + HEADER_SIZE + namesize);
        let data_end = data_offset + filesize;

        if data_end > INITRD_CPIO.len() {
            return None;
        }

        // Match on the entry name: try exact match, or strip `bin/` prefix.
        let entry_name_str = core::str::from_utf8(entry_name).ok()?;
        let base_name = entry_name_str.rsplit('/').next().unwrap_or(entry_name_str);

        if base_name == name {
            return Some(&INITRD_CPIO[data_offset..data_end]);
        }

        // Advance to the next entry: data end aligned to 4 bytes.
        offset = align4(data_end);
    }

    None
}

/// Parses an 8-character ASCII hex field from the CPIO header.
fn parse_hex_field(offset: usize, len: usize) -> Option<u64> {
    let field = &INITRD_CPIO[offset..offset + len];
    let mut value = 0u64;
    for &b in field {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        value = (value << 4) | u64::from(digit);
    }
    Some(value)
}

/// Aligns an offset up to the next 4-byte boundary.
const fn align4(offset: usize) -> usize {
    (offset + 3) & !3
}
