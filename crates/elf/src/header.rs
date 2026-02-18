//! ELF64 header parsing.
//!
//! Parses the ELF64 file header and program headers from raw byte slices
//! using safe field extraction via `from_le_bytes()`.

use core::fmt;

/// ELF magic bytes: `\x7fELF`.
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// ELF class: 64-bit.
const ELFCLASS64: u8 = 2;

/// ELF data encoding: little-endian.
const ELFDATA2LSB: u8 = 1;

/// ELF type: executable.
const ET_EXEC: u16 = 2;

/// ELF type: shared object (PIE).
const ET_DYN: u16 = 3;

/// ELF machine: x86-64.
const EM_X86_64: u16 = 62;

/// Program header type: loadable segment.
pub(crate) const PT_LOAD: u32 = 1;

/// Minimum size of an ELF64 file header (64 bytes).
const ELF64_EHDR_SIZE: usize = 64;

/// Size of an ELF64 program header entry (56 bytes).
pub(crate) const ELF64_PHDR_SIZE: usize = 56;

/// Size of an ELF64 section header entry (64 bytes).
pub(crate) const ELF64_SHDR_SIZE: usize = 64;

/// Read a little-endian `u16` from `data` at byte offset `off`.
///
/// # Panics
///
/// Panics if `off + 2 > data.len()`. Callers must bounds-check first.
pub(crate) fn le_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(*data[off..].first_chunk().unwrap())
}

/// Read a little-endian `u32` from `data` at byte offset `off`.
pub(crate) fn le_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(*data[off..].first_chunk().unwrap())
}

/// Read a little-endian `u64` from `data` at byte offset `off`.
pub(crate) fn le_u64(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(*data[off..].first_chunk().unwrap())
}

/// Errors that can occur when parsing an ELF file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    /// The file does not start with the ELF magic bytes.
    BadMagic,
    /// The ELF file is not 64-bit (`ELFCLASS64`).
    UnsupportedClass,
    /// The ELF file is not little-endian.
    UnsupportedEncoding,
    /// The ELF machine type is not `EM_X86_64`.
    UnsupportedMachine,
    /// The ELF type is not `ET_EXEC` or `ET_DYN`.
    UnsupportedType,
    /// The input data is too short for the declared structure.
    Truncated,
    /// A header offset or size is out of bounds.
    InvalidOffset,
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "invalid ELF magic bytes"),
            Self::UnsupportedClass => write!(f, "unsupported ELF class (expected ELFCLASS64)"),
            Self::UnsupportedEncoding => {
                write!(f, "unsupported data encoding (expected little-endian)")
            }
            Self::UnsupportedMachine => {
                write!(f, "unsupported machine type (expected EM_X86_64)")
            }
            Self::UnsupportedType => write!(f, "unsupported ELF type (expected ET_EXEC or ET_DYN)"),
            Self::Truncated => write!(f, "input data truncated"),
            Self::InvalidOffset => write!(f, "invalid header offset or size"),
        }
    }
}

/// Parsed ELF64 file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elf64Header {
    /// ELF type (`ET_EXEC` or `ET_DYN`).
    pub e_type: u16,
    /// Target machine architecture.
    pub e_machine: u16,
    /// Virtual address of the entry point.
    pub e_entry: u64,
    /// Offset of the program header table in the file.
    pub e_phoff: u64,
    /// Number of program header entries.
    pub e_phnum: u16,
    /// Size of each program header entry.
    pub e_phentsize: u16,
    /// Offset of the section header table in the file.
    pub e_shoff: u64,
    /// Size of each section header entry.
    pub e_shentsize: u16,
    /// Number of section header entries.
    pub e_shnum: u16,
    /// Section header string table index.
    pub e_shstrndx: u16,
}

impl Elf64Header {
    /// Parse an ELF64 file header from raw bytes.
    #[expect(clippy::similar_names, reason = "ELF spec naming convention")]
    ///
    /// Validates the magic, class, encoding, machine type, ELF type,
    /// and that the program header table fits within `data`.
    ///
    /// # Errors
    ///
    /// Returns [`ElfError`] if validation fails or the data is too short.
    pub fn parse(data: &[u8]) -> Result<Self, ElfError> {
        if data.len() < ELF64_EHDR_SIZE {
            return Err(ElfError::Truncated);
        }

        // Validate magic
        if data[..4] != ELF_MAGIC {
            return Err(ElfError::BadMagic);
        }

        // Validate class (byte 4) — must be ELFCLASS64
        if data[4] != ELFCLASS64 {
            return Err(ElfError::UnsupportedClass);
        }

        // Validate data encoding (byte 5) — must be little-endian
        if data[5] != ELFDATA2LSB {
            return Err(ElfError::UnsupportedEncoding);
        }

        // Parse fields — offsets are safe because we checked len >= 64 above
        let e_type = le_u16(data, 16);
        if e_type != ET_EXEC && e_type != ET_DYN {
            return Err(ElfError::UnsupportedType);
        }

        let e_machine = le_u16(data, 18);
        if e_machine != EM_X86_64 {
            return Err(ElfError::UnsupportedMachine);
        }

        let e_entry = le_u64(data, 24);
        let e_phoff = le_u64(data, 32);
        let e_shoff = le_u64(data, 40);
        let e_phentsize = le_u16(data, 54);
        let e_phnum = le_u16(data, 56);
        let e_shentsize = le_u16(data, 58);
        let e_shnum = le_u16(data, 60);
        let e_shstrndx = le_u16(data, 62);

        // Validate program header table bounds
        let ph_end = e_phoff
            .checked_add(u64::from(e_phnum) * u64::from(e_phentsize))
            .ok_or(ElfError::InvalidOffset)?;

        if ph_end > data.len() as u64 {
            return Err(ElfError::InvalidOffset);
        }

        // Validate program header entry size
        if e_phnum > 0 && (e_phentsize as usize) < ELF64_PHDR_SIZE {
            return Err(ElfError::InvalidOffset);
        }

        // Validate section header table bounds (if present)
        if e_shnum > 0 {
            if (e_shentsize as usize) < ELF64_SHDR_SIZE {
                return Err(ElfError::InvalidOffset);
            }
            let sh_end = e_shoff
                .checked_add(u64::from(e_shnum) * u64::from(e_shentsize))
                .ok_or(ElfError::InvalidOffset)?;
            if sh_end > data.len() as u64 {
                return Err(ElfError::InvalidOffset);
            }
        }

        Ok(Self {
            e_type,
            e_machine,
            e_entry,
            e_phoff,
            e_phnum,
            e_phentsize,
            e_shoff,
            e_shentsize,
            e_shnum,
            e_shstrndx,
        })
    }
}

/// Parsed ELF64 program header entry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Elf64ProgramHeader {
    /// Segment type.
    pub seg_type: u32,
    /// Segment flags (read/write/execute).
    pub flags: u32,
    /// Offset of the segment data in the file.
    pub offset: u64,
    /// Virtual address of the segment.
    pub vaddr: u64,
    /// Size of the segment data in the file.
    pub filesz: u64,
    /// Size of the segment in memory.
    pub memsz: u64,
}

impl Elf64ProgramHeader {
    /// Parse a program header entry from raw bytes at the given file offset.
    ///
    /// The caller must ensure `file_offset + ELF64_PHDR_SIZE <= data.len()`.
    pub(crate) fn parse(data: &[u8], file_offset: usize) -> Self {
        let b = &data[file_offset..];
        Self {
            seg_type: le_u32(b, 0),
            flags: le_u32(b, 4),
            offset: le_u64(b, 8),
            vaddr: le_u64(b, 16),
            // p_paddr at 24..32 — skipped
            filesz: le_u64(b, 32),
            memsz: le_u64(b, 40),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a minimal valid ELF64 header (64 bytes) as a `Vec<u8>`.
    ///
    /// Defaults: `ET_EXEC`, `EM_X86_64`, entry=0x401000, phoff=64, phnum=0, phentsize=56.
    /// Section header fields default to 0 (no sections).
    pub(crate) fn make_elf_header() -> Vec<u8> {
        let mut buf = vec![0u8; ELF64_EHDR_SIZE];

        // Magic
        buf[0..4].copy_from_slice(&ELF_MAGIC);
        // Class: ELFCLASS64
        buf[4] = ELFCLASS64;
        // Data: little-endian
        buf[5] = ELFDATA2LSB;
        // Version
        buf[6] = 1;
        // e_type: ET_EXEC
        buf[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
        // e_machine: EM_X86_64
        buf[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        // e_version
        buf[20..24].copy_from_slice(&1u32.to_le_bytes());
        // e_entry
        buf[24..32].copy_from_slice(&0x0040_1000u64.to_le_bytes());
        // e_phoff: right after header
        buf[32..40].copy_from_slice(&(ELF64_EHDR_SIZE as u64).to_le_bytes());
        // e_shoff: 0 (no sections by default) at offset 40..48
        // e_ehsize
        buf[52..54].copy_from_slice(&(ELF64_EHDR_SIZE as u16).to_le_bytes());
        // e_phentsize
        buf[54..56].copy_from_slice(&(ELF64_PHDR_SIZE as u16).to_le_bytes());
        // e_phnum: 0 (no program headers by default)
        buf[56..58].copy_from_slice(&0u16.to_le_bytes());
        // e_shentsize: default to ELF64_SHDR_SIZE
        buf[58..60].copy_from_slice(&(ELF64_SHDR_SIZE as u16).to_le_bytes());
        // e_shnum: 0 (no sections by default)
        buf[60..62].copy_from_slice(&0u16.to_le_bytes());
        // e_shstrndx: 0
        buf[62..64].copy_from_slice(&0u16.to_le_bytes());

        buf
    }

    /// Append a program header to the given ELF buffer.
    pub(crate) fn append_phdr(
        buf: &mut Vec<u8>,
        p_type: u32,
        p_flags: u32,
        p_offset: u64,
        p_vaddr: u64,
        p_filesz: u64,
        p_memsz: u64,
    ) {
        let start = buf.len();
        buf.resize(start + ELF64_PHDR_SIZE, 0);
        let b = &mut buf[start..];

        b[0..4].copy_from_slice(&p_type.to_le_bytes());
        b[4..8].copy_from_slice(&p_flags.to_le_bytes());
        b[8..16].copy_from_slice(&p_offset.to_le_bytes());
        b[16..24].copy_from_slice(&p_vaddr.to_le_bytes());
        // p_paddr at 24..32 — zero
        b[32..40].copy_from_slice(&p_filesz.to_le_bytes());
        b[40..48].copy_from_slice(&p_memsz.to_le_bytes());

        // Update e_phnum in the header
        let phnum = le_u16(buf, 56) + 1;
        buf[56..58].copy_from_slice(&phnum.to_le_bytes());
    }

    #[test]
    fn parse_valid_header() {
        let buf = make_elf_header();
        let hdr = Elf64Header::parse(&buf).expect("valid header");
        assert_eq!(hdr.e_type, ET_EXEC);
        assert_eq!(hdr.e_machine, EM_X86_64);
        assert_eq!(hdr.e_entry, 0x0040_1000);
        assert_eq!(hdr.e_phoff, ELF64_EHDR_SIZE as u64);
        assert_eq!(hdr.e_phnum, 0);
        assert_eq!(hdr.e_phentsize, ELF64_PHDR_SIZE as u16);
    }

    #[test]
    fn parse_dyn_type() {
        let mut buf = make_elf_header();
        buf[16..18].copy_from_slice(&ET_DYN.to_le_bytes());
        let hdr = Elf64Header::parse(&buf).expect("valid ET_DYN header");
        assert_eq!(hdr.e_type, ET_DYN);
    }

    #[test]
    fn reject_bad_magic() {
        let mut buf = make_elf_header();
        buf[0] = 0x00;
        assert_eq!(Elf64Header::parse(&buf), Err(ElfError::BadMagic));
    }

    #[test]
    fn reject_32bit_class() {
        let mut buf = make_elf_header();
        buf[4] = 1; // ELFCLASS32
        assert_eq!(Elf64Header::parse(&buf), Err(ElfError::UnsupportedClass));
    }

    #[test]
    fn reject_big_endian() {
        let mut buf = make_elf_header();
        buf[5] = 2; // ELFDATA2MSB
        assert_eq!(Elf64Header::parse(&buf), Err(ElfError::UnsupportedEncoding));
    }

    #[test]
    fn reject_wrong_machine() {
        let mut buf = make_elf_header();
        buf[18..20].copy_from_slice(&0x03u16.to_le_bytes()); // EM_386
        assert_eq!(Elf64Header::parse(&buf), Err(ElfError::UnsupportedMachine));
    }

    #[test]
    fn reject_unsupported_type() {
        let mut buf = make_elf_header();
        buf[16..18].copy_from_slice(&1u16.to_le_bytes()); // ET_REL
        assert_eq!(Elf64Header::parse(&buf), Err(ElfError::UnsupportedType));
    }

    #[test]
    fn reject_truncated_data() {
        let buf = vec![0u8; 32]; // Too short for a header
        assert_eq!(Elf64Header::parse(&buf), Err(ElfError::Truncated));
    }

    #[test]
    fn reject_truncated_empty() {
        assert_eq!(Elf64Header::parse(&[]), Err(ElfError::Truncated));
    }

    #[test]
    fn reject_phdr_out_of_bounds() {
        let mut buf = make_elf_header();
        // Set phnum=1 but don't append any program header data
        buf[56..58].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(Elf64Header::parse(&buf), Err(ElfError::InvalidOffset));
    }

    #[test]
    fn accept_header_with_phdr() {
        let mut buf = make_elf_header();
        append_phdr(&mut buf, PT_LOAD, 5, 120, 0x40_0000, 0x100, 0x200);
        let hdr = Elf64Header::parse(&buf).expect("valid header with phdr");
        assert_eq!(hdr.e_phnum, 1);
    }

    #[test]
    fn display_errors() {
        // Verify Display impl doesn't panic
        let errors = [
            ElfError::BadMagic,
            ElfError::UnsupportedClass,
            ElfError::UnsupportedEncoding,
            ElfError::UnsupportedMachine,
            ElfError::UnsupportedType,
            ElfError::Truncated,
            ElfError::InvalidOffset,
        ];
        for err in &errors {
            let msg = format!("{err}");
            assert!(!msg.is_empty());
        }
    }
}
