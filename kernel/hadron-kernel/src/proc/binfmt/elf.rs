//! ELF binary format handler.

use noalloc::vec::ArrayVec;

use super::{BinaryError, BinaryFormat, ExecImage, ExecSegment, SegmentFlags};

/// ELF segment permission flags.
const PF_X: u32 = 1;
const PF_W: u32 = 2;

/// Singleton handler for ELF64 binaries.
pub struct ElfHandler;

impl BinaryFormat for ElfHandler {
    fn name(&self) -> &'static str {
        "ELF"
    }

    fn probe(&self, data: &[u8]) -> bool {
        data.len() >= 4 && data[..4] == [0x7f, b'E', b'L', b'F']
    }

    fn load<'a>(&self, data: &'a [u8]) -> Result<ExecImage<'a>, BinaryError> {
        let elf = hadron_elf::ElfFile::parse(data).map_err(|e| match e {
            hadron_elf::ElfError::BadMagic => BinaryError::ParseError("bad ELF magic"),
            hadron_elf::ElfError::UnsupportedClass => {
                BinaryError::ParseError("unsupported ELF class")
            }
            hadron_elf::ElfError::UnsupportedEncoding => {
                BinaryError::ParseError("unsupported ELF encoding")
            }
            hadron_elf::ElfError::UnsupportedMachine => {
                BinaryError::ParseError("unsupported ELF machine")
            }
            hadron_elf::ElfError::UnsupportedType => {
                BinaryError::ParseError("unsupported ELF type")
            }
            hadron_elf::ElfError::Truncated => BinaryError::ParseError("truncated ELF"),
            hadron_elf::ElfError::InvalidOffset => {
                BinaryError::ParseError("invalid ELF offset")
            }
        })?;

        let mut segments = ArrayVec::new();
        for seg in elf.load_segments() {
            segments
                .try_push(ExecSegment {
                    vaddr: seg.vaddr,
                    data: seg.data,
                    memsz: seg.memsz,
                    flags: SegmentFlags {
                        writable: seg.flags & PF_W != 0,
                        executable: seg.flags & PF_X != 0,
                    },
                })
                .map_err(|_| BinaryError::TooManySegments)?;
        }

        Ok(ExecImage {
            entry_point: elf.entry_point(),
            segments,
        })
    }
}
