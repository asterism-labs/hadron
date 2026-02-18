//! ELF64 section header, symbol table, and string table parsing.
//!
//! Provides zero-copy, zero-allocation parsing of ELF64 section headers,
//! symbol tables (`.symtab`), and string tables (`.strtab`) from raw byte slices.

use crate::header::{ELF64_SHDR_SIZE, le_u16, le_u32, le_u64};

/// Section type: symbol table.
pub const SHT_SYMTAB: u32 = 2;

/// Section type: string table.
pub const SHT_STRTAB: u32 = 3;

/// Section type: relocation entries with addends.
pub const SHT_RELA: u32 = 4;

/// Section type: dynamic linking information.
pub const SHT_DYNAMIC: u32 = 6;

/// Section type: relocation entries without addends.
pub const SHT_REL: u32 = 9;

/// Section type: dynamic symbol table.
pub const SHT_DYNSYM: u32 = 11;

/// Symbol type: function.
pub const STT_FUNC: u8 = 2;

/// Symbol binding: global.
pub const STB_GLOBAL: u8 = 1;

/// Symbol binding: weak.
pub const STB_WEAK: u8 = 2;

/// Section flag: writable data.
pub const SHF_WRITE: u64 = 0x1;

/// Section flag: occupies memory during execution.
pub const SHF_ALLOC: u64 = 0x2;

/// Section flag: executable machine instructions.
pub const SHF_EXECINSTR: u64 = 0x4;

/// Section flag: `sh_info` contains a section header table index.
pub const SHF_INFO_LINK: u64 = 0x40;

/// Special section index: undefined.
pub const SHN_UNDEF: u16 = 0;

/// Size of an ELF64 symbol entry (24 bytes).
const ELF64_SYM_SIZE: usize = 24;

/// Parsed ELF64 section header entry.
#[derive(Debug, Clone, Copy)]
pub struct Elf64SectionHeader {
    /// Offset into the section header string table for this section's name.
    pub sh_name: u32,
    /// Section type (`SHT_SYMTAB`, `SHT_STRTAB`, etc.).
    pub sh_type: u32,
    /// Section flags.
    pub sh_flags: u64,
    /// Virtual address of the section in memory (0 for non-loaded sections).
    pub sh_addr: u64,
    /// File offset of the section data.
    pub sh_offset: u64,
    /// Size of the section data in bytes.
    pub sh_size: u64,
    /// Associated section index (e.g., `.strtab` index for `.symtab`).
    pub sh_link: u32,
    /// Extra info (interpretation depends on section type).
    pub sh_info: u32,
    /// Required alignment of the section (must be a power of two).
    pub sh_addralign: u64,
    /// Size of each entry (for sections with fixed-size entries).
    pub sh_entsize: u64,
}

impl Elf64SectionHeader {
    /// Parse a section header from raw bytes at the given file offset.
    ///
    /// The caller must ensure `file_offset + ELF64_SHDR_SIZE <= data.len()`.
    pub(crate) fn parse(data: &[u8], file_offset: usize) -> Self {
        let b = &data[file_offset..];
        Self {
            sh_name: le_u32(b, 0),
            sh_type: le_u32(b, 4),
            sh_flags: le_u64(b, 8),
            sh_addr: le_u64(b, 16),
            sh_offset: le_u64(b, 24),
            sh_size: le_u64(b, 32),
            sh_link: le_u32(b, 40),
            sh_info: le_u32(b, 44),
            sh_addralign: le_u64(b, 48),
            sh_entsize: le_u64(b, 56),
        }
    }
}

/// Parsed ELF64 symbol table entry.
#[derive(Debug, Clone, Copy)]
pub struct Elf64Symbol {
    /// Offset into the associated string table for this symbol's name.
    pub st_name: u32,
    /// Symbol type and binding packed into one byte.
    pub st_info: u8,
    /// Section index this symbol is defined in.
    pub st_shndx: u16,
    /// Symbol value (address for defined symbols).
    pub st_value: u64,
    /// Symbol size in bytes.
    pub st_size: u64,
}

impl Elf64Symbol {
    /// Parse a symbol entry from raw bytes at the given offset.
    ///
    /// The caller must ensure `offset + ELF64_SYM_SIZE <= data.len()`.
    fn parse(data: &[u8], offset: usize) -> Self {
        let b = &data[offset..];
        Self {
            st_name: le_u32(b, 0),
            st_info: b[4],
            // st_other at 5 — skipped
            st_shndx: le_u16(b, 6),
            st_value: le_u64(b, 8),
            st_size: le_u64(b, 16),
        }
    }

    /// Returns the symbol type (lower 4 bits of `st_info`).
    #[must_use]
    pub fn sym_type(&self) -> u8 {
        self.st_info & 0xf
    }

    /// Returns the symbol binding (upper 4 bits of `st_info`).
    #[must_use]
    pub fn sym_bind(&self) -> u8 {
        self.st_info >> 4
    }
}

/// A zero-copy wrapper around a NUL-terminated string table section.
#[derive(Debug, Clone, Copy)]
pub struct StringTable<'a> {
    data: &'a [u8],
}

impl<'a> StringTable<'a> {
    /// Creates a new string table from the raw section data.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Looks up a NUL-terminated string at the given byte offset.
    ///
    /// Returns `None` if the offset is out of bounds or the string
    /// contains invalid UTF-8.
    #[must_use]
    pub fn get(&self, offset: u32) -> Option<&'a str> {
        let start = offset as usize;
        if start >= self.data.len() {
            return None;
        }
        let remaining = &self.data[start..];
        let nul_pos = remaining.iter().position(|&b| b == 0)?;
        core::str::from_utf8(&remaining[..nul_pos]).ok()
    }
}

/// An iterator over ELF64 section headers.
pub struct SectionIter<'a> {
    data: &'a [u8],
    shoff: usize,
    shentsize: usize,
    index: usize,
    count: usize,
}

impl Iterator for SectionIter<'_> {
    type Item = Elf64SectionHeader;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.count {
            return None;
        }
        let offset = self.shoff + self.index * self.shentsize;
        if offset + ELF64_SHDR_SIZE > self.data.len() {
            return None;
        }
        let hdr = Elf64SectionHeader::parse(self.data, offset);
        self.index += 1;
        Some(hdr)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.count.saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

/// An iterator over ELF64 symbol table entries.
pub struct SymbolIter<'a> {
    data: &'a [u8],
    offset: usize,
    end: usize,
}

impl Iterator for SymbolIter<'_> {
    type Item = Elf64Symbol;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset + ELF64_SYM_SIZE > self.end {
            return None;
        }
        let sym = Elf64Symbol::parse(self.data, self.offset);
        self.offset += ELF64_SYM_SIZE;
        Some(sym)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.end - self.offset) / ELF64_SYM_SIZE;
        (remaining, Some(remaining))
    }
}

// ---------------------------------------------------------------------------
// ElfFile section/symbol methods
// ---------------------------------------------------------------------------

use crate::reloc::RelaIter;
use crate::segment::ElfFile;

impl<'a> ElfFile<'a> {
    /// Returns an iterator over all section headers.
    ///
    /// Returns an empty iterator if the ELF has no sections (`e_shnum == 0`).
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "ELF fields are u32/u64, truncation checked by format"
    )]
    pub fn sections(&self) -> SectionIter<'a> {
        let hdr = self.header();
        SectionIter {
            data: self.raw_data(),
            shoff: hdr.e_shoff as usize,
            shentsize: hdr.e_shentsize as usize,
            index: 0,
            count: hdr.e_shnum as usize,
        }
    }

    /// Finds the first section header with the given type.
    #[must_use]
    pub fn find_section_by_type(&self, sh_type: u32) -> Option<Elf64SectionHeader> {
        self.sections().find(|s| s.sh_type == sh_type)
    }

    /// Finds a section by name, looking up names in the section header string table.
    #[must_use]
    pub fn find_section_by_name(&self, name: &str) -> Option<Elf64SectionHeader> {
        let shstrtab = self.section_header_strtab()?;
        self.sections()
            .find(|s| shstrtab.get(s.sh_name) == Some(name))
    }

    /// Returns the raw data slice for a given section header.
    ///
    /// Returns `None` if the section data is out of bounds.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "ELF fields are u32/u64, truncation checked by format"
    )]
    pub fn section_data(&self, shdr: &Elf64SectionHeader) -> Option<&'a [u8]> {
        let start = shdr.sh_offset as usize;
        let size = shdr.sh_size as usize;
        let data = self.raw_data();
        if start.checked_add(size)? > data.len() {
            return None;
        }
        Some(&data[start..start + size])
    }

    /// Returns an iterator over symbols in the given section (must be `SHT_SYMTAB` or `SHT_DYNSYM`).
    ///
    /// Returns `None` if the section data is out of bounds.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "ELF fields are u32/u64, truncation checked by format"
    )]
    pub fn symbols(&self, shdr: &Elf64SectionHeader) -> Option<SymbolIter<'a>> {
        let data = self.section_data(shdr)?;
        let base = shdr.sh_offset as usize;
        Some(SymbolIter {
            data: self.raw_data(),
            offset: base,
            end: base + data.len(),
        })
    }

    /// Returns the string table associated with a symbol table section (via `sh_link`).
    #[must_use]
    pub fn linked_strtab(&self, symtab: &Elf64SectionHeader) -> Option<StringTable<'a>> {
        let hdr = self.header();
        let link = symtab.sh_link as usize;
        if link >= hdr.e_shnum as usize {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "ELF fields are u32/u64, truncation checked by format"
        )]
        let offset = hdr.e_shoff as usize + link * hdr.e_shentsize as usize;
        let data = self.raw_data();
        if offset + ELF64_SHDR_SIZE > data.len() {
            return None;
        }
        let strtab_shdr = Elf64SectionHeader::parse(data, offset);
        let strtab_data = self.section_data(&strtab_shdr)?;
        Some(StringTable::new(strtab_data))
    }

    /// Returns an iterator over `Rela` entries from a `SHT_RELA` section.
    ///
    /// Returns `None` if the section data is out of bounds.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "ELF fields are u32/u64, truncation checked by format"
    )]
    pub fn rela_entries(&self, shdr: &Elf64SectionHeader) -> Option<RelaIter<'a>> {
        let data = self.section_data(shdr)?;
        let base = shdr.sh_offset as usize;
        Some(RelaIter::new(self.raw_data(), base, base + data.len()))
    }

    /// Returns the section header at the given 0-based index.
    ///
    /// Returns `None` if the index is out of range or the section header
    /// is out of bounds in the file.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "ELF fields are u32/u64, truncation checked by format"
    )]
    pub fn section_by_index(&self, index: usize) -> Option<Elf64SectionHeader> {
        let hdr = self.header();
        if index >= hdr.e_shnum as usize {
            return None;
        }
        let offset = hdr.e_shoff as usize + index * hdr.e_shentsize as usize;
        let data = self.raw_data();
        if offset + ELF64_SHDR_SIZE > data.len() {
            return None;
        }
        Some(Elf64SectionHeader::parse(data, offset))
    }

    /// Returns an iterator over sections with the `SHF_ALLOC` flag set,
    /// yielding `(section_index, header)` pairs.
    ///
    /// Useful for `ET_REL` loading where allocatable sections must be placed
    /// in memory.
    pub fn alloc_sections(&self) -> impl Iterator<Item = (usize, Elf64SectionHeader)> + 'a {
        self.sections()
            .enumerate()
            .filter(|(_, s)| s.sh_flags & SHF_ALLOC != 0)
    }

    /// Returns an iterator over `SHT_RELA` section headers.
    pub fn rela_sections(&self) -> impl Iterator<Item = Elf64SectionHeader> + 'a {
        self.sections().filter(|s| s.sh_type == SHT_RELA)
    }

    /// Returns the section header string table (`.shstrtab`).
    fn section_header_strtab(&self) -> Option<StringTable<'a>> {
        let hdr = self.header();
        if hdr.e_shstrndx == 0 || hdr.e_shstrndx >= hdr.e_shnum {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "ELF fields are u32/u64, truncation checked by format"
        )]
        let offset = hdr.e_shoff as usize + hdr.e_shstrndx as usize * hdr.e_shentsize as usize;
        let data = self.raw_data();
        if offset + ELF64_SHDR_SIZE > data.len() {
            return None;
        }
        let shdr = Elf64SectionHeader::parse(data, offset);
        let strtab_data = self.section_data(&shdr)?;
        Some(StringTable::new(strtab_data))
    }

    /// Returns the underlying raw ELF data.
    #[must_use]
    pub fn raw_data(&self) -> &'a [u8] {
        self.data
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests::make_elf_header;

    /// Size of an ELF64 section header entry.
    const SHDR_SIZE: usize = ELF64_SHDR_SIZE;
    /// Size of an ELF64 symbol entry.
    const SYM_SIZE: usize = ELF64_SYM_SIZE;

    /// Append a section header to the ELF buffer and bump `e_shnum`.
    pub(crate) fn append_section(
        buf: &mut Vec<u8>,
        sh_name: u32,
        sh_type: u32,
        sh_flags: u64,
        sh_addr: u64,
        sh_offset: u64,
        sh_size: u64,
        sh_link: u32,
        sh_info: u32,
        sh_addralign: u64,
        sh_entsize: u64,
    ) {
        let start = buf.len();
        buf.resize(start + SHDR_SIZE, 0);
        let b = &mut buf[start..];

        b[0..4].copy_from_slice(&sh_name.to_le_bytes());
        b[4..8].copy_from_slice(&sh_type.to_le_bytes());
        b[8..16].copy_from_slice(&sh_flags.to_le_bytes());
        b[16..24].copy_from_slice(&sh_addr.to_le_bytes());
        b[24..32].copy_from_slice(&sh_offset.to_le_bytes());
        b[32..40].copy_from_slice(&sh_size.to_le_bytes());
        b[40..44].copy_from_slice(&sh_link.to_le_bytes());
        b[44..48].copy_from_slice(&sh_info.to_le_bytes());
        b[48..56].copy_from_slice(&sh_addralign.to_le_bytes());
        b[56..64].copy_from_slice(&sh_entsize.to_le_bytes());

        // Update e_shnum
        let shnum = le_u16(buf, 60) + 1;
        buf[60..62].copy_from_slice(&shnum.to_le_bytes());
    }

    /// Build a symbol entry as raw bytes.
    fn make_symbol(
        st_name: u32,
        st_info: u8,
        st_shndx: u16,
        st_value: u64,
        st_size: u64,
    ) -> [u8; 24] {
        let mut b = [0u8; SYM_SIZE];
        b[0..4].copy_from_slice(&st_name.to_le_bytes());
        b[4] = st_info;
        b[6..8].copy_from_slice(&st_shndx.to_le_bytes());
        b[8..16].copy_from_slice(&st_value.to_le_bytes());
        b[16..24].copy_from_slice(&st_size.to_le_bytes());
        b
    }

    /// Build a test ELF with sections: NULL, .strtab, .symtab, .shstrtab.
    fn make_elf_with_symtab() -> Vec<u8> {
        let mut buf = make_elf_header();

        // String table data: "\0hello\0world\0"
        let strtab_data = b"\0hello\0world\0";

        // Symbol table: one null symbol + one function symbol
        // st_info for STT_FUNC | STB_GLOBAL: (1 << 4) | 2 = 0x12
        let sym0 = make_symbol(0, 0, 0, 0, 0); // null symbol
        let sym1 = make_symbol(1, 0x12, 1, 0x1000, 0x42); // hello, FUNC, GLOBAL

        // Section header string table: "\0.strtab\0.symtab\0.shstrtab\0"
        let shstrtab_data = b"\0.strtab\0.symtab\0.shstrtab\0";

        // Layout:
        //   offset 64: section headers (4 sections * 64 = 256 bytes)
        //   offset 320: strtab data (13 bytes)
        //   offset 333: symtab data (48 bytes)
        //   offset 381: shstrtab data (28 bytes)
        let shdr_start = 64u64;
        let strtab_off = shdr_start + 4 * SHDR_SIZE as u64;
        let symtab_off = strtab_off + strtab_data.len() as u64;
        let shstrtab_off = symtab_off + (SYM_SIZE * 2) as u64;

        // Set e_shoff in header
        buf[40..48].copy_from_slice(&shdr_start.to_le_bytes());
        // Set e_shstrndx to 3 (index of .shstrtab)
        buf[62..64].copy_from_slice(&3u16.to_le_bytes());

        // Section 0: NULL
        append_section(&mut buf, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);

        // Section 1: .strtab (SHT_STRTAB)
        append_section(
            &mut buf,
            1, // name offset in shstrtab: ".strtab"
            SHT_STRTAB,
            0,             // sh_flags
            0,             // sh_addr
            strtab_off,
            strtab_data.len() as u64,
            0,             // sh_link
            0,             // sh_info
            1,             // sh_addralign
            0,             // sh_entsize
        );

        // Section 2: .symtab (SHT_SYMTAB), sh_link=1 (points to .strtab)
        append_section(
            &mut buf,
            9, // name offset in shstrtab: ".symtab"
            SHT_SYMTAB,
            0,             // sh_flags
            0,             // sh_addr
            symtab_off,
            (SYM_SIZE * 2) as u64,
            1,             // sh_link -> .strtab
            0,             // sh_info
            8,             // sh_addralign
            SYM_SIZE as u64,
        );

        // Section 3: .shstrtab (SHT_STRTAB)
        append_section(
            &mut buf,
            17, // name offset in shstrtab: ".shstrtab"
            SHT_STRTAB,
            0,             // sh_flags
            0,             // sh_addr
            shstrtab_off,
            shstrtab_data.len() as u64,
            0,             // sh_link
            0,             // sh_info
            1,             // sh_addralign
            0,             // sh_entsize
        );

        // Append actual data
        buf.extend_from_slice(strtab_data);
        buf.extend_from_slice(&sym0);
        buf.extend_from_slice(&sym1);
        buf.extend_from_slice(shstrtab_data);

        buf
    }

    #[test]
    fn no_sections_yields_empty_iterator() {
        let buf = make_elf_header();
        let elf = ElfFile::parse(&buf).expect("valid ELF");
        assert_eq!(elf.sections().count(), 0);
    }

    #[test]
    fn section_iteration() {
        let buf = make_elf_with_symtab();
        let elf = ElfFile::parse(&buf).expect("valid ELF");
        let sections: Vec<_> = elf.sections().collect();

        assert_eq!(sections.len(), 4);
        assert_eq!(sections[0].sh_type, 0); // NULL
        assert_eq!(sections[1].sh_type, SHT_STRTAB);
        assert_eq!(sections[2].sh_type, SHT_SYMTAB);
        assert_eq!(sections[3].sh_type, SHT_STRTAB);
    }

    #[test]
    fn find_section_by_type() {
        let buf = make_elf_with_symtab();
        let elf = ElfFile::parse(&buf).expect("valid ELF");

        let symtab = elf.find_section_by_type(SHT_SYMTAB);
        assert!(symtab.is_some());
        assert_eq!(symtab.unwrap().sh_type, SHT_SYMTAB);
        assert_eq!(symtab.unwrap().sh_link, 1);

        let dynsym = elf.find_section_by_type(SHT_DYNSYM);
        assert!(dynsym.is_none());
    }

    #[test]
    fn find_section_by_name() {
        let buf = make_elf_with_symtab();
        let elf = ElfFile::parse(&buf).expect("valid ELF");

        let symtab = elf.find_section_by_name(".symtab");
        assert!(symtab.is_some());
        assert_eq!(symtab.unwrap().sh_type, SHT_SYMTAB);

        let strtab = elf.find_section_by_name(".strtab");
        assert!(strtab.is_some());
        assert_eq!(strtab.unwrap().sh_type, SHT_STRTAB);

        let missing = elf.find_section_by_name(".debug_info");
        assert!(missing.is_none());
    }

    #[test]
    fn symbol_parsing() {
        let buf = make_elf_with_symtab();
        let elf = ElfFile::parse(&buf).expect("valid ELF");

        let symtab = elf.find_section_by_type(SHT_SYMTAB).unwrap();
        let symbols: Vec<_> = elf.symbols(&symtab).unwrap().collect();

        assert_eq!(symbols.len(), 2);

        // Null symbol
        assert_eq!(symbols[0].st_name, 0);
        assert_eq!(symbols[0].st_value, 0);

        // Function symbol
        assert_eq!(symbols[1].st_name, 1);
        assert_eq!(symbols[1].st_value, 0x1000);
        assert_eq!(symbols[1].st_size, 0x42);
        assert_eq!(symbols[1].sym_type(), STT_FUNC);
        assert_eq!(symbols[1].sym_bind(), STB_GLOBAL);
    }

    #[test]
    fn linked_strtab_lookup() {
        let buf = make_elf_with_symtab();
        let elf = ElfFile::parse(&buf).expect("valid ELF");

        let symtab = elf.find_section_by_type(SHT_SYMTAB).unwrap();
        let strtab = elf.linked_strtab(&symtab).unwrap();

        assert_eq!(strtab.get(0), Some(""));
        assert_eq!(strtab.get(1), Some("hello"));
        assert_eq!(strtab.get(7), Some("world"));
    }

    #[test]
    fn string_table_out_of_bounds() {
        let strtab = StringTable::new(b"\0hello\0");
        assert_eq!(strtab.get(100), None);
    }

    #[test]
    fn string_table_no_nul_terminator() {
        let strtab = StringTable::new(b"abc");
        // No NUL terminator — should return None
        assert_eq!(strtab.get(0), None);
    }

    #[test]
    fn section_data_bounds_check() {
        let buf = make_elf_with_symtab();
        let elf = ElfFile::parse(&buf).expect("valid ELF");

        let strtab_shdr = elf.find_section_by_type(SHT_STRTAB).unwrap();
        let data = elf.section_data(&strtab_shdr);
        assert!(data.is_some());
        assert_eq!(data.unwrap().len(), 13); // "\0hello\0world\0"
    }

    #[test]
    fn raw_data_accessor() {
        let buf = make_elf_header();
        let elf = ElfFile::parse(&buf).expect("valid ELF");
        assert_eq!(elf.raw_data().len(), buf.len());
    }
}
