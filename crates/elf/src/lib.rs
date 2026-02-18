//! Minimal ELF64 parser for Hadron OS.
//!
//! Parses ELF64 headers and `PT_LOAD` segments from raw byte slices using
//! safe field extraction (`from_le_bytes`). No unsafe code, no allocations.
//!
//! # Usage
//!
//! ```
//! use hadron_elf::ElfFile;
//!
//! fn load_elf(data: &[u8]) {
//!     let elf = ElfFile::parse(data).expect("valid ELF");
//!     let entry = elf.entry_point();
//!     for seg in elf.load_segments() {
//!         // Map seg.data at seg.vaddr, zero-fill to seg.memsz
//!     }
//! }
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod header;
pub mod reloc;
pub mod section;
pub mod segment;

pub use header::{Elf64Header, ElfError, ElfType};
pub use reloc::{
    Elf64Rela, RelaIter, RelocError, RelocValue, compute_x86_64_reloc, R_X86_64_32,
    R_X86_64_32S, R_X86_64_64, R_X86_64_GLOB_DAT, R_X86_64_NONE, R_X86_64_PC32,
    R_X86_64_PLT32, R_X86_64_RELATIVE,
};
pub use section::{
    Elf64SectionHeader, Elf64Symbol, SHF_ALLOC, SHF_EXECINSTR, SHF_WRITE, SHN_UNDEF, SHT_DYNSYM,
    SHT_RELA, SHT_STRTAB, SHT_SYMTAB, STB_GLOBAL, STB_WEAK, STT_FUNC, StringTable,
};
pub use segment::{ElfFile, LoadSegment};
