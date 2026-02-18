//! ELF binary format handler.
//!
//! Supports `ET_EXEC` (fixed-address) and `ET_DYN` (static-PIE, relocated to
//! [`USER_PIE_BASE`]). `ET_REL` is rejected here — relocatable objects are
//! loaded via a separate module loader path.

use hadron_elf::ElfType;
use noalloc::vec::ArrayVec;

use super::{BinaryError, BinaryFormat, ExecImage, ExecSegment, SegmentFlags};

/// ELF segment permission flags.
const PF_X: u32 = 1;
/// ELF segment permission flags.
const PF_W: u32 = 2;

/// Default base address for PIE (`ET_DYN`) user binaries.
const USER_PIE_BASE: u64 = 0x40_0000;

/// Singleton handler for ELF64 binaries.
pub struct ElfHandler;

/// Convert an ELF parse error into a [`BinaryError`].
fn map_elf_error(e: hadron_elf::ElfError) -> BinaryError {
    match e {
        hadron_elf::ElfError::BadMagic => BinaryError::ParseError("bad ELF magic"),
        hadron_elf::ElfError::UnsupportedClass => BinaryError::ParseError("unsupported ELF class"),
        hadron_elf::ElfError::UnsupportedEncoding => {
            BinaryError::ParseError("unsupported ELF encoding")
        }
        hadron_elf::ElfError::UnsupportedMachine => {
            BinaryError::ParseError("unsupported ELF machine")
        }
        hadron_elf::ElfError::UnsupportedType => BinaryError::ParseError("unsupported ELF type"),
        hadron_elf::ElfError::Truncated => BinaryError::ParseError("truncated ELF"),
        hadron_elf::ElfError::InvalidOffset => BinaryError::ParseError("invalid ELF offset"),
    }
}

/// Collect `PT_LOAD` segments into an [`ArrayVec`], optionally offsetting
/// vaddrs by `base_addr`.
fn collect_segments<'a>(
    elf: &hadron_elf::ElfFile<'a>,
    base_addr: u64,
) -> Result<ArrayVec<ExecSegment<'a>, 16>, BinaryError> {
    let mut segments = ArrayVec::new();
    for seg in elf.load_segments() {
        segments
            .try_push(ExecSegment {
                vaddr: seg.vaddr + base_addr,
                data: seg.data,
                memsz: seg.memsz,
                flags: SegmentFlags {
                    writable: seg.flags & PF_W != 0,
                    executable: seg.flags & PF_X != 0,
                },
            })
            .map_err(|_| BinaryError::TooManySegments)?;
    }
    Ok(segments)
}

/// Load an `ET_EXEC` binary. Segments map at their stated vaddrs.
fn load_exec<'a>(elf: &hadron_elf::ElfFile<'a>) -> Result<ExecImage<'a>, BinaryError> {
    let segments = collect_segments(elf, 0)?;
    Ok(ExecImage {
        entry_point: elf.entry_point(),
        base_addr: 0,
        needs_relocation: false,
        elf_data: None,
        segments,
    })
}

/// Load an `ET_DYN` (static-PIE) binary. Segments are relocated to
/// [`USER_PIE_BASE`] and the image is marked for relocation application.
fn load_dyn<'a>(
    elf: &hadron_elf::ElfFile<'a>,
    data: &'a [u8],
) -> Result<ExecImage<'a>, BinaryError> {
    let base = USER_PIE_BASE;
    let segments = collect_segments(elf, base)?;
    Ok(ExecImage {
        entry_point: base + elf.entry_point(),
        base_addr: base,
        needs_relocation: true,
        elf_data: Some(data),
        segments,
    })
}

impl BinaryFormat for ElfHandler {
    fn name(&self) -> &'static str {
        "ELF"
    }

    fn probe(&self, data: &[u8]) -> bool {
        data.len() >= 4 && data[..4] == [0x7f, b'E', b'L', b'F']
    }

    fn load<'a>(&self, data: &'a [u8]) -> Result<ExecImage<'a>, BinaryError> {
        let elf = hadron_elf::ElfFile::parse(data).map_err(map_elf_error)?;

        match elf.elf_type() {
            ElfType::Exec => load_exec(&elf),
            ElfType::Dyn => load_dyn(&elf, data),
            ElfType::Rel => Err(BinaryError::Unimplemented(
                "ET_REL via BinaryFormat; use ModuleLoader",
            )),
        }
    }
}
