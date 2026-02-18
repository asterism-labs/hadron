//! HBTF (Hadron Backtrace Format) generator.
//!
//! Extracts function symbols and DWARF line info from the kernel ELF binary
//! and writes a compact binary format for runtime backtrace symbolication.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

/// HBTF file magic bytes.
const HBTF_MAGIC: [u8; 4] = *b"HBTF";

/// HBTF format version.
const HBTF_VERSION: u32 = 1;

/// HBTF file header size in bytes.
const HEADER_SIZE: u32 = 32;

/// Size of a symbol entry in the HBTF binary.
const SYM_ENTRY_SIZE: usize = 20;

/// Size of a line entry in the HBTF binary.
const LINE_ENTRY_SIZE: usize = 16;

/// A function symbol extracted from the ELF.
struct FuncSymbol {
    /// Offset from kernel virtual base.
    addr: u64,
    /// Symbol size in bytes.
    size: u32,
    /// Demangled function name.
    name: String,
}

/// A line info entry extracted from DWARF.
struct LineInfo {
    /// Offset from kernel virtual base.
    addr: u64,
    /// Source file path.
    file: String,
    /// Line number.
    line: u32,
}

/// A deduplicated string pool for the HBTF binary.
struct StringPool {
    data: Vec<u8>,
    offsets: HashMap<String, u32>,
}

impl StringPool {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            offsets: HashMap::new(),
        }
    }

    /// Inserts a string into the pool, deduplicating. Returns the offset.
    fn insert(&mut self, s: &str) -> u32 {
        if let Some(&offset) = self.offsets.get(s) {
            return offset;
        }
        let offset = self.data.len() as u32;
        self.data.extend_from_slice(s.as_bytes());
        self.data.push(0); // NUL terminator
        self.offsets.insert(s.to_string(), offset);
        offset
    }
}

/// Generates an HBTF file from a kernel ELF binary.
///
/// If `include_lines` is true, DWARF `.debug_line` info is included
/// (typically for debug builds only).
pub fn generate_hbtf(kernel_elf: &Path, output: &Path, include_lines: bool) -> Result<()> {
    let elf_data =
        std::fs::read(kernel_elf).with_context(|| format!("reading {}", kernel_elf.display()))?;

    let elf =
        hadron_elf::ElfFile::parse(&elf_data).map_err(|e| anyhow::anyhow!("parsing ELF: {e}"))?;

    // Compute the kernel virtual base from the lowest PT_LOAD segment so that
    // HBTF stores offsets (matching what the runtime backtrace code queries).
    let kernel_virt_base = elf
        .load_segments()
        .map(|seg| seg.vaddr)
        .min()
        .unwrap_or(elf.entry_point());

    println!("HBTF: kernel_virt_base = {kernel_virt_base:#x}");

    // Extract function symbols
    let mut symbols = extract_symbols(&elf, kernel_virt_base);
    symbols.sort_by_key(|s| s.addr);

    // Extract line info (if requested and available)
    let mut lines = if include_lines {
        extract_lines(&elf, kernel_virt_base)
    } else {
        Vec::new()
    };
    lines.sort_by_key(|l| l.addr);

    // Build string pool and write HBTF
    let mut pool = StringPool::new();

    // Pre-insert all strings
    let sym_name_offsets: Vec<u32> = symbols.iter().map(|s| pool.insert(&s.name)).collect();
    let line_file_offsets: Vec<u32> = lines.iter().map(|l| pool.insert(&l.file)).collect();

    // Compute offsets
    let sym_offset = HEADER_SIZE;
    let sym_table_size = (symbols.len() * SYM_ENTRY_SIZE) as u32;
    let line_offset = sym_offset + sym_table_size;
    let line_table_size = (lines.len() * LINE_ENTRY_SIZE) as u32;
    let strings_offset = line_offset + line_table_size;

    // Build output buffer
    let total_size = strings_offset as usize + pool.data.len();
    let mut buf = Vec::with_capacity(total_size);

    // File header (32 bytes)
    buf.extend_from_slice(&HBTF_MAGIC);
    buf.extend_from_slice(&HBTF_VERSION.to_le_bytes());
    buf.extend_from_slice(&(symbols.len() as u32).to_le_bytes());
    buf.extend_from_slice(&sym_offset.to_le_bytes());
    buf.extend_from_slice(&(lines.len() as u32).to_le_bytes());
    buf.extend_from_slice(&line_offset.to_le_bytes());
    buf.extend_from_slice(&strings_offset.to_le_bytes());
    buf.extend_from_slice(&(pool.data.len() as u32).to_le_bytes());
    assert_eq!(buf.len(), HEADER_SIZE as usize);

    // Symbol table (sorted by addr)
    for (sym, &name_off) in symbols.iter().zip(sym_name_offsets.iter()) {
        buf.extend_from_slice(&sym.addr.to_le_bytes()); // 8 bytes
        buf.extend_from_slice(&sym.size.to_le_bytes()); // 4 bytes
        buf.extend_from_slice(&name_off.to_le_bytes()); // 4 bytes
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved, 4 bytes
    }

    // Line table (sorted by addr)
    for (line, &file_off) in lines.iter().zip(line_file_offsets.iter()) {
        buf.extend_from_slice(&line.addr.to_le_bytes()); // 8 bytes
        buf.extend_from_slice(&file_off.to_le_bytes()); // 4 bytes
        buf.extend_from_slice(&line.line.to_le_bytes()); // 4 bytes
    }

    // String pool
    buf.extend_from_slice(&pool.data);

    assert_eq!(buf.len(), total_size);

    std::fs::write(output, &buf).with_context(|| format!("writing {}", output.display()))?;

    println!(
        "HBTF: {} symbols, {} lines, {} bytes -> {}",
        symbols.len(),
        lines.len(),
        total_size,
        output.display()
    );

    Ok(())
}

/// Extract function symbols from the ELF symbol table.
///
/// Addresses are stored as offsets from `kernel_virt_base` so that the
/// runtime backtrace code (which queries by offset) can find them.
fn extract_symbols(elf: &hadron_elf::ElfFile<'_>, kernel_virt_base: u64) -> Vec<FuncSymbol> {
    let symtab = match elf.find_section_by_type(hadron_elf::SHT_SYMTAB) {
        Some(s) => s,
        None => return Vec::new(),
    };

    let strtab = match elf.linked_strtab(&symtab) {
        Some(s) => s,
        None => return Vec::new(),
    };

    let syms = match elf.symbols(&symtab) {
        Some(s) => s,
        None => return Vec::new(),
    };

    let mut result = Vec::new();

    for sym in syms {
        // Only include defined function symbols
        if sym.sym_type() != hadron_elf::STT_FUNC {
            continue;
        }
        if sym.st_shndx == hadron_elf::SHN_UNDEF {
            continue;
        }
        if sym.st_value == 0 {
            continue;
        }
        // Skip symbols below the kernel virtual base (non-kernel symbols).
        if sym.st_value < kernel_virt_base {
            continue;
        }

        let raw_name = match strtab.get(sym.st_name) {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };

        // Demangle the symbol name
        let demangled = format!("{:#}", rustc_demangle::demangle(raw_name));

        result.push(FuncSymbol {
            addr: sym.st_value.wrapping_sub(kernel_virt_base),
            size: sym.st_size as u32,
            name: demangled,
        });
    }

    result
}

/// Extract line info from DWARF `.debug_line` section.
///
/// Addresses are stored as offsets from `kernel_virt_base` so that the
/// runtime backtrace code (which queries by offset) can find them.
fn extract_lines(elf: &hadron_elf::ElfFile<'_>, kernel_virt_base: u64) -> Vec<LineInfo> {
    let debug_line = match elf.find_section_by_name(".debug_line") {
        Some(s) => s,
        None => return Vec::new(),
    };

    let line_data = match elf.section_data(&debug_line) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let mut result = Vec::new();

    for unit in hadron_dwarf::DebugLine::new(line_data) {
        let header = unit.header();
        for row in unit.rows() {
            if row.end_sequence || row.line == 0 {
                continue;
            }

            // Build file path
            let file_path = match header.file(row.file_index) {
                Some(file) => {
                    let dir = header.directory(file.directory_index).unwrap_or("");
                    if dir.is_empty() {
                        file.name.to_string()
                    } else {
                        format!("{dir}/{}", file.name)
                    }
                }
                None => continue,
            };

            // Simplify paths: strip everything up to and including the workspace root
            let simplified = simplify_path(&file_path);

            // Skip lines below the kernel virtual base (non-kernel code).
            if row.address < kernel_virt_base {
                continue;
            }

            result.push(LineInfo {
                addr: row.address.wrapping_sub(kernel_virt_base),
                file: simplified,
                line: row.line,
            });
        }
    }

    // Deduplicate: keep only the first entry for each address
    result.sort_by_key(|l| l.addr);
    result.dedup_by_key(|l| l.addr);

    result
}

/// Simplify a source file path by stripping everything before the crate directory.
///
/// Turns paths like `/home/user/.cargo/registry/.../src/lib.rs` into `src/lib.rs`
/// and `/home/user/projects/hadron/kernel/hadron-kernel/src/boot.rs` into
/// `kernel/hadron-kernel/src/boot.rs`.
fn simplify_path(path: &str) -> String {
    // Look for known markers in the path
    for marker in &["kernel/", "crates/"] {
        if let Some(pos) = path.find(marker) {
            return path[pos..].to_string();
        }
    }
    // For external crate paths, try to find "src/" and keep from there
    if let Some(pos) = path.rfind("/src/") {
        // Walk backwards to find the crate name
        let before_src = &path[..pos];
        if let Some(crate_pos) = before_src.rfind('/') {
            return path[crate_pos + 1..].to_string();
        }
        return path[pos + 1..].to_string();
    }
    // Last resort: just the filename
    if let Some(pos) = path.rfind('/') {
        return path[pos + 1..].to_string();
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal HBTF blob with known symbols and lines for testing.
    ///
    /// Symbols: "fn_alpha" at 0x1000 (size 0x100),
    ///          "fn_beta"  at 0x2000 (size 0x200),
    ///          "fn_gamma" at 0x5000 (size 0x80)
    /// Lines:   0x1042 -> "boot.rs":10, 0x2010 -> "main.rs":55
    fn build_test_hbtf() -> Vec<u8> {
        let mut pool = StringPool::new();

        let symbols: &[(&str, u64, u32)] = &[
            ("fn_alpha", 0x1000, 0x100),
            ("fn_beta", 0x2000, 0x200),
            ("fn_gamma", 0x5000, 0x80),
        ];

        let lines: &[(&str, u64, u32)] = &[("boot.rs", 0x1042, 10), ("main.rs", 0x2010, 55)];

        let sym_name_offsets: Vec<u32> = symbols
            .iter()
            .map(|(name, _, _)| pool.insert(name))
            .collect();
        let line_file_offsets: Vec<u32> =
            lines.iter().map(|(file, _, _)| pool.insert(file)).collect();

        let sym_offset = HEADER_SIZE;
        let sym_table_size = (symbols.len() * SYM_ENTRY_SIZE) as u32;
        let line_offset = sym_offset + sym_table_size;
        let line_table_size = (lines.len() * LINE_ENTRY_SIZE) as u32;
        let strings_offset = line_offset + line_table_size;

        let total_size = strings_offset as usize + pool.data.len();
        let mut buf = Vec::with_capacity(total_size);

        // Header (32 bytes)
        buf.extend_from_slice(&HBTF_MAGIC);
        buf.extend_from_slice(&HBTF_VERSION.to_le_bytes());
        buf.extend_from_slice(&(symbols.len() as u32).to_le_bytes());
        buf.extend_from_slice(&sym_offset.to_le_bytes());
        buf.extend_from_slice(&(lines.len() as u32).to_le_bytes());
        buf.extend_from_slice(&line_offset.to_le_bytes());
        buf.extend_from_slice(&strings_offset.to_le_bytes());
        buf.extend_from_slice(&(pool.data.len() as u32).to_le_bytes());

        // Symbol table
        for ((_, addr, size), &name_off) in symbols.iter().zip(sym_name_offsets.iter()) {
            buf.extend_from_slice(&addr.to_le_bytes());
            buf.extend_from_slice(&size.to_le_bytes());
            buf.extend_from_slice(&name_off.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
        }

        // Line table
        for ((_, addr, line), &file_off) in lines.iter().zip(line_file_offsets.iter()) {
            buf.extend_from_slice(&addr.to_le_bytes());
            buf.extend_from_slice(&file_off.to_le_bytes());
            buf.extend_from_slice(&line.to_le_bytes());
        }

        // String pool
        buf.extend_from_slice(&pool.data);

        assert_eq!(buf.len(), total_size);
        buf
    }

    // ----- Test-only HBTF readers (mirrors kernel parsing logic) -----

    fn test_read_nul_str(data: &[u8], offset: usize) -> Option<&str> {
        if offset >= data.len() {
            return None;
        }
        let remaining = &data[offset..];
        let nul_pos = remaining.iter().position(|&b| b == 0)?;
        core::str::from_utf8(&remaining[..nul_pos]).ok()
    }

    fn test_lookup_symbol(hbtf: &[u8], offset: u64) -> Option<(String, u64)> {
        let sym_count = u32::from_le_bytes(hbtf[8..12].try_into().unwrap()) as usize;
        let sym_offset = u32::from_le_bytes(hbtf[12..16].try_into().unwrap()) as usize;
        let strings_offset = u32::from_le_bytes(hbtf[24..28].try_into().unwrap()) as usize;

        if sym_count == 0 {
            return None;
        }

        // Binary search: find the last symbol with addr <= offset
        let mut lo = 0usize;
        let mut hi = sym_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let entry_off = sym_offset + mid * SYM_ENTRY_SIZE;
            let sym_addr = u64::from_le_bytes(hbtf[entry_off..entry_off + 8].try_into().unwrap());
            if sym_addr <= offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo == 0 {
            return None;
        }

        let idx = lo - 1;
        let entry_off = sym_offset + idx * SYM_ENTRY_SIZE;
        let sym_addr = u64::from_le_bytes(hbtf[entry_off..entry_off + 8].try_into().unwrap());
        let sym_size = u32::from_le_bytes(hbtf[entry_off + 8..entry_off + 12].try_into().unwrap());
        let name_off =
            u32::from_le_bytes(hbtf[entry_off + 12..entry_off + 16].try_into().unwrap()) as usize;

        let func_offset = offset - sym_addr;
        if sym_size > 0 && func_offset >= u64::from(sym_size) {
            return None;
        }

        let name_start = strings_offset + name_off;
        let name = test_read_nul_str(hbtf, name_start)?;

        Some((name.to_string(), func_offset))
    }

    fn test_lookup_line(hbtf: &[u8], offset: u64) -> Option<(String, u32)> {
        let line_count = u32::from_le_bytes(hbtf[16..20].try_into().unwrap()) as usize;
        let line_offset = u32::from_le_bytes(hbtf[20..24].try_into().unwrap()) as usize;
        let strings_offset = u32::from_le_bytes(hbtf[24..28].try_into().unwrap()) as usize;

        if line_count == 0 {
            return None;
        }

        // Binary search: find the last line entry with addr <= offset
        let mut lo = 0usize;
        let mut hi = line_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let entry_off = line_offset + mid * LINE_ENTRY_SIZE;
            let line_addr = u64::from_le_bytes(hbtf[entry_off..entry_off + 8].try_into().unwrap());
            if line_addr <= offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo == 0 {
            return None;
        }

        let idx = lo - 1;
        let entry_off = line_offset + idx * LINE_ENTRY_SIZE;
        let file_off =
            u32::from_le_bytes(hbtf[entry_off + 8..entry_off + 12].try_into().unwrap()) as usize;
        let line_num = u32::from_le_bytes(hbtf[entry_off + 12..entry_off + 16].try_into().unwrap());

        let file_start = strings_offset + file_off;
        let file = test_read_nul_str(hbtf, file_start)?;

        Some((file.to_string(), line_num))
    }

    // ----- Unit tests -----

    #[test]
    fn test_string_pool_dedup() {
        let mut pool = StringPool::new();
        let off1 = pool.insert("hello");
        let off2 = pool.insert("hello");
        assert_eq!(off1, off2);
    }

    #[test]
    fn test_string_pool_nul_terminated() {
        let mut pool = StringPool::new();
        pool.insert("abc");
        assert_eq!(pool.data.len(), 4); // "abc" + NUL
        assert_eq!(&pool.data, &[b'a', b'b', b'c', 0]);
    }

    #[test]
    fn test_hbtf_header() {
        let hbtf = build_test_hbtf();

        // Magic
        assert_eq!(&hbtf[0..4], b"HBTF");

        // Version
        let version = u32::from_le_bytes(hbtf[4..8].try_into().unwrap());
        assert_eq!(version, 1);

        // Symbol count and offset
        let sym_count = u32::from_le_bytes(hbtf[8..12].try_into().unwrap());
        assert_eq!(sym_count, 3);
        let sym_offset = u32::from_le_bytes(hbtf[12..16].try_into().unwrap());
        assert_eq!(sym_offset, HEADER_SIZE);

        // Line count and offset
        let line_count = u32::from_le_bytes(hbtf[16..20].try_into().unwrap());
        assert_eq!(line_count, 2);
        let line_offset = u32::from_le_bytes(hbtf[20..24].try_into().unwrap());
        assert_eq!(line_offset, HEADER_SIZE + 3 * SYM_ENTRY_SIZE as u32);

        // Strings offset
        let strings_offset = u32::from_le_bytes(hbtf[24..28].try_into().unwrap());
        assert_eq!(strings_offset, line_offset + 2 * LINE_ENTRY_SIZE as u32);
    }

    #[test]
    fn test_lookup_symbol_exact() {
        let hbtf = build_test_hbtf();
        let result = test_lookup_symbol(&hbtf, 0x1000);
        assert_eq!(result, Some(("fn_alpha".to_string(), 0)));
    }

    #[test]
    fn test_lookup_symbol_within() {
        let hbtf = build_test_hbtf();
        let result = test_lookup_symbol(&hbtf, 0x1042);
        assert_eq!(result, Some(("fn_alpha".to_string(), 0x42)));
    }

    #[test]
    fn test_lookup_symbol_between() {
        let hbtf = build_test_hbtf();
        // fn_alpha ends at 0x1100 (0x1000 + 0x100), fn_beta starts at 0x2000
        let result = test_lookup_symbol(&hbtf, 0x1500);
        assert_eq!(result, None);
    }

    #[test]
    fn test_lookup_symbol_before_first() {
        let hbtf = build_test_hbtf();
        let result = test_lookup_symbol(&hbtf, 0x500);
        assert_eq!(result, None);
    }

    #[test]
    fn test_lookup_line_exact() {
        let hbtf = build_test_hbtf();
        let result = test_lookup_line(&hbtf, 0x1042);
        assert_eq!(result, Some(("boot.rs".to_string(), 10)));
    }

    #[test]
    fn test_lookup_line_between() {
        let hbtf = build_test_hbtf();
        // Between 0x1042 and 0x2010, should return the earlier entry
        let result = test_lookup_line(&hbtf, 0x1500);
        assert_eq!(result, Some(("boot.rs".to_string(), 10)));
    }

    #[test]
    fn test_simplify_path() {
        // kernel/ marker
        assert_eq!(
            simplify_path("/home/user/hadron/kernel/hadron-kernel/src/boot.rs"),
            "kernel/hadron-kernel/src/boot.rs"
        );
        // crates/ marker
        assert_eq!(
            simplify_path("/home/user/hadron/crates/noalloc/src/lib.rs"),
            "crates/noalloc/src/lib.rs"
        );
        // External crate with /src/
        assert_eq!(
            simplify_path("/home/user/.cargo/registry/src/bitflags-2.0/src/lib.rs"),
            "bitflags-2.0/src/lib.rs"
        );
        // Just a filename
        assert_eq!(simplify_path("lib.rs"), "lib.rs");
    }

    #[test]
    fn test_empty_hbtf() {
        let sym_offset = HEADER_SIZE;
        let line_offset = sym_offset;
        let strings_offset = line_offset;

        let mut buf = Vec::with_capacity(HEADER_SIZE as usize);

        // Header with zero symbols and zero lines
        buf.extend_from_slice(&HBTF_MAGIC);
        buf.extend_from_slice(&HBTF_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // sym_count
        buf.extend_from_slice(&sym_offset.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // line_count
        buf.extend_from_slice(&line_offset.to_le_bytes());
        buf.extend_from_slice(&strings_offset.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // strings_size

        assert_eq!(buf.len(), HEADER_SIZE as usize);

        // Header is valid
        assert_eq!(&buf[0..4], b"HBTF");

        // Lookups return None
        assert_eq!(test_lookup_symbol(&buf, 0x1000), None);
        assert_eq!(test_lookup_line(&buf, 0x1000), None);
    }
}
