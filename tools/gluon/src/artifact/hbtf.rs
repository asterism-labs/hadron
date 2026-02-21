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
pub(crate) const SYM_ENTRY_SIZE: usize = 20;

/// Size of a line entry in the HBTF binary.
pub(crate) const LINE_ENTRY_SIZE: usize = 16;

/// A function symbol extracted from the ELF.
pub struct FuncSymbol {
    /// Offset from kernel virtual base.
    pub addr: u64,
    /// Symbol size in bytes.
    pub size: u32,
    /// Demangled function name.
    pub name: String,
}

/// A line info entry extracted from DWARF.
pub(crate) struct LineInfo {
    /// Offset from kernel virtual base.
    pub(crate) addr: u64,
    /// Source file path.
    pub(crate) file: String,
    /// Line number.
    pub(crate) line: u32,
}

/// A deduplicated string pool for the HBTF binary.
pub(crate) struct StringPool {
    pub(crate) data: Vec<u8>,
    offsets: HashMap<String, u32>,
}

impl StringPool {
    pub(crate) fn new() -> Self {
        Self {
            data: Vec::new(),
            offsets: HashMap::new(),
        }
    }

    /// Inserts a string into the pool, deduplicating. Returns the offset.
    pub(crate) fn insert(&mut self, s: &str) -> u32 {
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

    println!("  HBTF: kernel_virt_base = {kernel_virt_base:#x}");

    // Extract function symbols.
    let mut symbols = extract_symbols(&elf, kernel_virt_base);
    symbols.sort_by_key(|s| s.addr);

    // Extract line info (if requested and available).
    let mut lines = if include_lines {
        extract_lines(&elf, kernel_virt_base)
    } else {
        Vec::new()
    };
    lines.sort_by_key(|l| l.addr);

    // Build string pool and write HBTF.
    let mut pool = StringPool::new();

    // Pre-insert all strings.
    let sym_name_offsets: Vec<u32> = symbols.iter().map(|s| pool.insert(&s.name)).collect();
    let line_file_offsets: Vec<u32> = lines.iter().map(|l| pool.insert(&l.file)).collect();

    // Compute offsets.
    let sym_offset = HEADER_SIZE;
    let sym_table_size = (symbols.len() * SYM_ENTRY_SIZE) as u32;
    let line_offset = sym_offset + sym_table_size;
    let line_table_size = (lines.len() * LINE_ENTRY_SIZE) as u32;
    let strings_offset = line_offset + line_table_size;

    // Build output buffer.
    let total_size = strings_offset as usize + pool.data.len();
    let mut buf = Vec::with_capacity(total_size);

    // File header (32 bytes).
    buf.extend_from_slice(&HBTF_MAGIC);
    buf.extend_from_slice(&HBTF_VERSION.to_le_bytes());
    buf.extend_from_slice(&(symbols.len() as u32).to_le_bytes());
    buf.extend_from_slice(&sym_offset.to_le_bytes());
    buf.extend_from_slice(&(lines.len() as u32).to_le_bytes());
    buf.extend_from_slice(&line_offset.to_le_bytes());
    buf.extend_from_slice(&strings_offset.to_le_bytes());
    buf.extend_from_slice(&(pool.data.len() as u32).to_le_bytes());
    assert_eq!(buf.len(), HEADER_SIZE as usize);

    // Symbol table (sorted by addr).
    for (sym, &name_off) in symbols.iter().zip(sym_name_offsets.iter()) {
        buf.extend_from_slice(&sym.addr.to_le_bytes()); // 8 bytes
        buf.extend_from_slice(&sym.size.to_le_bytes()); // 4 bytes
        buf.extend_from_slice(&name_off.to_le_bytes()); // 4 bytes
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved, 4 bytes
    }

    // Line table (sorted by addr).
    for (line, &file_off) in lines.iter().zip(line_file_offsets.iter()) {
        buf.extend_from_slice(&line.addr.to_le_bytes()); // 8 bytes
        buf.extend_from_slice(&file_off.to_le_bytes()); // 4 bytes
        buf.extend_from_slice(&line.line.to_le_bytes()); // 4 bytes
    }

    // String pool.
    buf.extend_from_slice(&pool.data);

    assert_eq!(buf.len(), total_size);

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, &buf).with_context(|| format!("writing {}", output.display()))?;

    println!(
        "  HBTF: {} symbols, {} lines, {} bytes -> {}",
        symbols.len(),
        lines.len(),
        total_size,
        output.display()
    );

    Ok(())
}

/// Extract function symbols from the ELF symbol table.
pub fn extract_symbols(elf: &hadron_elf::ElfFile<'_>, kernel_virt_base: u64) -> Vec<FuncSymbol> {
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
        // Only include defined function symbols.
        if sym.sym_type() != hadron_elf::STT_FUNC {
            continue;
        }
        if sym.st_shndx == hadron_elf::SHN_UNDEF {
            continue;
        }
        if sym.st_value == 0 {
            continue;
        }
        // Skip symbols below the kernel virtual base.
        if sym.st_value < kernel_virt_base {
            continue;
        }

        let raw_name = match strtab.get(sym.st_name) {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };

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
pub(crate) fn extract_lines(elf: &hadron_elf::ElfFile<'_>, kernel_virt_base: u64) -> Vec<LineInfo> {
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

            let simplified = simplify_path(&file_path);

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

    // Deduplicate: keep only the first entry for each address.
    result.sort_by_key(|l| l.addr);
    result.dedup_by_key(|l| l.addr);

    result
}

/// Simplify a source file path by stripping everything before the crate directory.
pub(crate) fn simplify_path(path: &str) -> String {
    for marker in &["kernel/", "crates/"] {
        if let Some(pos) = path.find(marker) {
            return path[pos..].to_string();
        }
    }
    if let Some(pos) = path.rfind("/src/") {
        let before_src = &path[..pos];
        if let Some(crate_pos) = before_src.rfind('/') {
            return path[crate_pos + 1..].to_string();
        }
        return path[pos + 1..].to_string();
    }
    if let Some(pos) = path.rfind('/') {
        return path[pos + 1..].to_string();
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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

        buf.extend_from_slice(&HBTF_MAGIC);
        buf.extend_from_slice(&HBTF_VERSION.to_le_bytes());
        buf.extend_from_slice(&(symbols.len() as u32).to_le_bytes());
        buf.extend_from_slice(&sym_offset.to_le_bytes());
        buf.extend_from_slice(&(lines.len() as u32).to_le_bytes());
        buf.extend_from_slice(&line_offset.to_le_bytes());
        buf.extend_from_slice(&strings_offset.to_le_bytes());
        buf.extend_from_slice(&(pool.data.len() as u32).to_le_bytes());

        for ((_, addr, size), &name_off) in symbols.iter().zip(sym_name_offsets.iter()) {
            buf.extend_from_slice(&addr.to_le_bytes());
            buf.extend_from_slice(&size.to_le_bytes());
            buf.extend_from_slice(&name_off.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
        }

        for ((_, addr, line), &file_off) in lines.iter().zip(line_file_offsets.iter()) {
            buf.extend_from_slice(&addr.to_le_bytes());
            buf.extend_from_slice(&file_off.to_le_bytes());
            buf.extend_from_slice(&line.to_le_bytes());
        }

        buf.extend_from_slice(&pool.data);
        assert_eq!(buf.len(), total_size);
        buf
    }

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

    #[test]
    fn string_pool_dedup() {
        let mut pool = StringPool::new();
        let off1 = pool.insert("hello");
        let off2 = pool.insert("hello");
        assert_eq!(off1, off2);
    }

    #[test]
    fn string_pool_nul_terminated() {
        let mut pool = StringPool::new();
        pool.insert("abc");
        assert_eq!(pool.data.len(), 4);
        assert_eq!(&pool.data, &[b'a', b'b', b'c', 0]);
    }

    #[test]
    fn hbtf_header() {
        let hbtf = build_test_hbtf();
        assert_eq!(&hbtf[0..4], b"HBTF");
        assert_eq!(u32::from_le_bytes(hbtf[4..8].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(hbtf[8..12].try_into().unwrap()), 3);
        assert_eq!(
            u32::from_le_bytes(hbtf[12..16].try_into().unwrap()),
            HEADER_SIZE
        );
        assert_eq!(u32::from_le_bytes(hbtf[16..20].try_into().unwrap()), 2);
    }

    #[test]
    fn lookup_symbol_exact() {
        let hbtf = build_test_hbtf();
        assert_eq!(
            test_lookup_symbol(&hbtf, 0x1000),
            Some(("fn_alpha".to_string(), 0))
        );
    }

    #[test]
    fn lookup_symbol_within() {
        let hbtf = build_test_hbtf();
        assert_eq!(
            test_lookup_symbol(&hbtf, 0x1042),
            Some(("fn_alpha".to_string(), 0x42))
        );
    }

    #[test]
    fn lookup_symbol_between() {
        let hbtf = build_test_hbtf();
        assert_eq!(test_lookup_symbol(&hbtf, 0x1500), None);
    }

    #[test]
    fn lookup_symbol_before_first() {
        let hbtf = build_test_hbtf();
        assert_eq!(test_lookup_symbol(&hbtf, 0x500), None);
    }

    #[test]
    fn lookup_line_exact() {
        let hbtf = build_test_hbtf();
        assert_eq!(
            test_lookup_line(&hbtf, 0x1042),
            Some(("boot.rs".to_string(), 10))
        );
    }

    #[test]
    fn lookup_line_between() {
        let hbtf = build_test_hbtf();
        assert_eq!(
            test_lookup_line(&hbtf, 0x1500),
            Some(("boot.rs".to_string(), 10))
        );
    }

    #[test]
    fn path_simplification() {
        assert_eq!(
            simplify_path("/home/user/hadron/kernel/hadron-kernel/src/boot.rs"),
            "kernel/hadron-kernel/src/boot.rs"
        );
        assert_eq!(
            simplify_path("/home/user/hadron/crates/noalloc/src/lib.rs"),
            "crates/noalloc/src/lib.rs"
        );
        assert_eq!(
            simplify_path("/home/user/.cargo/registry/src/bitflags-2.0/src/lib.rs"),
            "bitflags-2.0/src/lib.rs"
        );
        assert_eq!(simplify_path("lib.rs"), "lib.rs");
    }

    #[test]
    fn empty_hbtf() {
        let sym_offset = HEADER_SIZE;
        let line_offset = sym_offset;
        let strings_offset = line_offset;

        let mut buf = Vec::with_capacity(HEADER_SIZE as usize);
        buf.extend_from_slice(&HBTF_MAGIC);
        buf.extend_from_slice(&HBTF_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&sym_offset.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&line_offset.to_le_bytes());
        buf.extend_from_slice(&strings_offset.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());

        assert_eq!(&buf[0..4], b"HBTF");
        assert_eq!(test_lookup_symbol(&buf, 0x1000), None);
        assert_eq!(test_lookup_line(&buf, 0x1000), None);
    }
}
