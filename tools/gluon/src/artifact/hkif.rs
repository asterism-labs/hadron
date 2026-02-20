//! HKIF (Hadron Kernel Image Format) generator.
//!
//! Embeds backtrace data (symbols, lines, strings) and kernel metadata directly
//! into the kernel ELF as a custom linker section (`.hadron_hkif`). This replaces
//! the separate HBTF boot module.
//!
//! The build uses a two-pass link: pass 1 produces a kernel with an empty
//! `.hadron_hkif` section, then this module extracts symbols/lines, serializes
//! the HKIF blob, assembles it into an object file, and triggers pass 2 to
//! embed the populated section.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::hbtf;

// ---------------------------------------------------------------------------
// HKIF constants
// ---------------------------------------------------------------------------

/// HKIF magic bytes.
const HKIF_MAGIC: [u8; 4] = *b"HKIF";

/// HKIF format version.
const HKIF_VERSION: u16 = 1;

/// Flag: backtrace sections present.
const FLAG_HAS_BACKTRACE: u16 = 1 << 1;

/// HKIF header size in bytes.
const HEADER_SIZE: usize = 64;

/// Section directory entry size in bytes.
const DIR_ENTRY_SIZE: usize = 16;

/// Section type: symbols.
const SECTION_SYMBOLS: u32 = 2;

/// Section type: lines.
const SECTION_LINES: u32 = 3;

/// Section type: strings.
const SECTION_STRINGS: u32 = 4;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate an HKIF binary blob from a kernel ELF (pass 1 output).
///
/// Extracts symbols, line info, and kernel metadata, then serializes the
/// complete HKIF blob to `output`.
pub fn generate_hkif(kernel_elf: &Path, output: &Path, include_lines: bool) -> Result<()> {
    let elf_data =
        std::fs::read(kernel_elf).with_context(|| format!("reading {}", kernel_elf.display()))?;
    let elf =
        hadron_elf::ElfFile::parse(&elf_data).map_err(|e| anyhow::anyhow!("parsing ELF: {e}"))?;

    // Kernel metadata from PT_LOAD segments.
    let kernel_virt_base = elf
        .load_segments()
        .map(|seg| seg.vaddr)
        .min()
        .unwrap_or(elf.entry_point());

    let kernel_image_size = elf
        .load_segments()
        .map(|seg| seg.vaddr + seg.memsz)
        .max()
        .unwrap_or(kernel_virt_base)
        .saturating_sub(kernel_virt_base);

    let entry_point = elf.entry_point();

    println!("  HKIF: kernel_virt_base = {kernel_virt_base:#x}, image_size = {kernel_image_size:#x}, entry = {entry_point:#x}");

    // Extract symbols and lines using shared HBTF helpers.
    let mut symbols = hbtf::extract_symbols(&elf, kernel_virt_base);
    symbols.sort_by_key(|s| s.addr);

    let mut lines = if include_lines {
        hbtf::extract_lines(&elf, kernel_virt_base)
    } else {
        Vec::new()
    };
    lines.sort_by_key(|l| l.addr);

    // Build string pool.
    let mut pool = hbtf::StringPool::new();
    let sym_name_offsets: Vec<u32> = symbols.iter().map(|s| pool.insert(&s.name)).collect();
    let line_file_offsets: Vec<u32> = lines.iter().map(|l| pool.insert(&l.file)).collect();

    // Compute section data blobs.
    let sym_data = serialize_symbols(&symbols, &sym_name_offsets);
    let line_data = serialize_lines(&lines, &line_file_offsets);
    let string_data = &pool.data;

    // Count sections (only non-empty ones).
    let mut sections: Vec<(u32, &[u8])> = Vec::new();
    if !sym_data.is_empty() {
        sections.push((SECTION_SYMBOLS, &sym_data));
    }
    if !line_data.is_empty() {
        sections.push((SECTION_LINES, &line_data));
    }
    if !string_data.is_empty() {
        sections.push((SECTION_STRINGS, string_data));
    }

    let section_count = sections.len() as u32;
    let directory_offset = HEADER_SIZE as u32;
    let directory_size = section_count as usize * DIR_ENTRY_SIZE;
    let data_start = HEADER_SIZE + directory_size;

    // Compute per-section offsets.
    let mut current_offset = data_start;
    let mut dir_entries: Vec<(u32, u32, u32)> = Vec::new(); // (type, offset, size)
    for &(sec_type, data) in &sections {
        dir_entries.push((sec_type, current_offset as u32, data.len() as u32));
        current_offset += data.len();
    }

    let total_size = current_offset;

    // Flags.
    let mut flags: u16 = 0;
    if !symbols.is_empty() || !lines.is_empty() {
        flags |= FLAG_HAS_BACKTRACE;
    }

    // Serialize the blob.
    let mut buf = Vec::with_capacity(total_size);

    // Header (64 bytes).
    buf.extend_from_slice(&HKIF_MAGIC);                          // 0x00: magic (4)
    buf.extend_from_slice(&HKIF_VERSION.to_le_bytes());           // 0x04: version (2)
    buf.extend_from_slice(&flags.to_le_bytes());                  // 0x06: flags (2)
    buf.extend_from_slice(&section_count.to_le_bytes());          // 0x08: section_count (4)
    buf.extend_from_slice(&directory_offset.to_le_bytes());       // 0x0C: directory_offset (4)
    buf.extend_from_slice(&kernel_virt_base.to_le_bytes());       // 0x10: kernel_virt_base (8)
    buf.extend_from_slice(&kernel_image_size.to_le_bytes());      // 0x18: kernel_image_size (8)
    buf.extend_from_slice(&entry_point.to_le_bytes());            // 0x20: entry_point (8)
    buf.extend_from_slice(&[0u8; 16]);                            // 0x28: reserved (16)
    buf.extend_from_slice(&(total_size as u32).to_le_bytes());    // 0x38: total_size (4)
    buf.extend_from_slice(&0u32.to_le_bytes());                   // 0x3C: checksum placeholder (4)
    assert_eq!(buf.len(), HEADER_SIZE);

    // Section directory.
    for &(sec_type, offset, size) in &dir_entries {
        buf.extend_from_slice(&sec_type.to_le_bytes());   // type (4)
        buf.extend_from_slice(&offset.to_le_bytes());     // offset (4)
        buf.extend_from_slice(&size.to_le_bytes());       // size (4)
        buf.extend_from_slice(&0u32.to_le_bytes());       // reserved (4)
    }

    // Section data blobs.
    for &(_, data) in &sections {
        buf.extend_from_slice(data);
    }

    assert_eq!(buf.len(), total_size);

    // Compute CRC-32 with the checksum field zeroed (it already is).
    let crc = crc32fast::hash(&buf);
    buf[0x3C..0x40].copy_from_slice(&crc.to_le_bytes());

    // Write output.
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, &buf).with_context(|| format!("writing {}", output.display()))?;

    println!(
        "  HKIF: {} symbols, {} lines, {} bytes -> {}",
        symbols.len(),
        lines.len(),
        total_size,
        output.display()
    );

    Ok(())
}

/// Generate a `.S` assembly file that `.incbin`s the HKIF blob into `.hadron_hkif`.
pub fn generate_hkif_asm(hkif_bin: &Path, output: &Path) -> Result<()> {
    let asm = format!(
        r#".section .hadron_hkif, "a", @progbits
.globl __hadron_hkif_payload
__hadron_hkif_payload:
    .incbin "{}"
"#,
        hkif_bin.display()
    );

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, asm).with_context(|| format!("writing {}", output.display()))?;
    Ok(())
}

/// Assemble the HKIF `.S` file into an object file using `clang`.
pub fn assemble_hkif(asm_path: &Path, obj_path: &Path) -> Result<()> {
    let output = Command::new("clang")
        .arg("--target=x86_64-unknown-none-elf")
        .arg("-c")
        .arg(asm_path)
        .arg("-o")
        .arg(obj_path)
        .output()
        .context("failed to run clang for HKIF assembly")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("clang failed to assemble HKIF:\n{stderr}");
    }

    Ok(())
}

/// Return the extra link object path for the HKIF object file.
pub fn hkif_object_path(root: &Path) -> PathBuf {
    root.join("build/hkif.o")
}

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

/// Serialize symbol entries (20 bytes each).
fn serialize_symbols(symbols: &[hbtf::FuncSymbol], name_offsets: &[u32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(symbols.len() * hbtf::SYM_ENTRY_SIZE);
    for (sym, &name_off) in symbols.iter().zip(name_offsets.iter()) {
        buf.extend_from_slice(&sym.addr.to_le_bytes());   // 8
        buf.extend_from_slice(&sym.size.to_le_bytes());   // 4
        buf.extend_from_slice(&name_off.to_le_bytes());   // 4
        buf.extend_from_slice(&0u32.to_le_bytes());       // 4 reserved
    }
    buf
}

/// Serialize line entries (16 bytes each).
fn serialize_lines(lines: &[hbtf::LineInfo], file_offsets: &[u32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines.len() * hbtf::LINE_ENTRY_SIZE);
    for (line, &file_off) in lines.iter().zip(file_offsets.iter()) {
        buf.extend_from_slice(&line.addr.to_le_bytes());  // 8
        buf.extend_from_slice(&file_off.to_le_bytes());   // 4
        buf.extend_from_slice(&line.line.to_le_bytes());   // 4
    }
    buf
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hkif_header_layout() {
        // Build a minimal HKIF blob with known data.
        let mut pool = hbtf::StringPool::new();
        let symbols = vec![hbtf::FuncSymbol {
            addr: 0x1000,
            size: 0x100,
            name: "test_fn".to_string(),
        }];
        let sym_name_offsets: Vec<u32> = symbols.iter().map(|s| pool.insert(&s.name)).collect();
        let sym_data = serialize_symbols(&symbols, &sym_name_offsets);
        let string_data = &pool.data;

        let sections: Vec<(u32, &[u8])> = vec![
            (SECTION_SYMBOLS, &sym_data),
            (SECTION_STRINGS, string_data),
        ];

        let section_count = sections.len() as u32;
        let directory_offset = HEADER_SIZE as u32;
        let directory_size = section_count as usize * DIR_ENTRY_SIZE;
        let data_start = HEADER_SIZE + directory_size;

        let mut current_offset = data_start;
        let mut dir_entries: Vec<(u32, u32, u32)> = Vec::new();
        for &(sec_type, data) in &sections {
            dir_entries.push((sec_type, current_offset as u32, data.len() as u32));
            current_offset += data.len();
        }

        let total_size = current_offset;
        let mut buf = Vec::with_capacity(total_size);

        // Header
        buf.extend_from_slice(&HKIF_MAGIC);
        buf.extend_from_slice(&HKIF_VERSION.to_le_bytes());
        buf.extend_from_slice(&FLAG_HAS_BACKTRACE.to_le_bytes());
        buf.extend_from_slice(&section_count.to_le_bytes());
        buf.extend_from_slice(&directory_offset.to_le_bytes());
        buf.extend_from_slice(&0xFFFF_FFFF_8000_0000u64.to_le_bytes()); // virt base
        buf.extend_from_slice(&0x10000u64.to_le_bytes()); // image size
        buf.extend_from_slice(&0xFFFF_FFFF_8000_1000u64.to_le_bytes()); // entry
        buf.extend_from_slice(&[0u8; 16]); // reserved
        buf.extend_from_slice(&(total_size as u32).to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // checksum placeholder

        // Directory
        for &(sec_type, offset, size) in &dir_entries {
            buf.extend_from_slice(&sec_type.to_le_bytes());
            buf.extend_from_slice(&offset.to_le_bytes());
            buf.extend_from_slice(&size.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
        }

        // Data
        for &(_, data) in &sections {
            buf.extend_from_slice(data);
        }

        assert_eq!(buf.len(), total_size);

        // Verify header fields.
        assert_eq!(&buf[0..4], b"HKIF");
        assert_eq!(u16::from_le_bytes(buf[4..6].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(buf[6..8].try_into().unwrap()), FLAG_HAS_BACKTRACE);
        assert_eq!(u32::from_le_bytes(buf[8..12].try_into().unwrap()), 2); // 2 sections
        assert_eq!(u32::from_le_bytes(buf[12..16].try_into().unwrap()), 64); // dir at 64

        // Verify directory entry for symbols.
        let dir_base = HEADER_SIZE;
        let sym_type = u32::from_le_bytes(buf[dir_base..dir_base + 4].try_into().unwrap());
        assert_eq!(sym_type, SECTION_SYMBOLS);
    }

    #[test]
    fn hkif_checksum_nonzero() {
        // Minimal HKIF with no sections.
        let total_size = HEADER_SIZE;
        let mut buf = vec![0u8; total_size];
        buf[0..4].copy_from_slice(&HKIF_MAGIC);
        buf[4..6].copy_from_slice(&HKIF_VERSION.to_le_bytes());
        buf[0x38..0x3C].copy_from_slice(&(total_size as u32).to_le_bytes());
        // checksum at 0x3C is 0 — compute CRC over that.
        let crc = crc32fast::hash(&buf);
        buf[0x3C..0x40].copy_from_slice(&crc.to_le_bytes());

        let stored_crc = u32::from_le_bytes(buf[0x3C..0x40].try_into().unwrap());
        assert_ne!(stored_crc, 0);

        // Verify: zeroing checksum field and rehashing should match.
        let mut verify_buf = buf.clone();
        verify_buf[0x3C..0x40].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(crc32fast::hash(&verify_buf), stored_crc);
    }
}
