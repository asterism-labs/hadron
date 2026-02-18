//! ELF64 segment (program header) iteration.
//!
//! Provides [`ElfFile`] as the main entry point for parsing an ELF64 binary,
//! and [`LoadSegment`] for iterating over `PT_LOAD` segments.

use crate::header::{ELF64_PHDR_SIZE, Elf64Header, Elf64ProgramHeader, ElfError, PT_LOAD};

/// A parsed ELF64 file, holding a reference to the raw data and the parsed header.
#[derive(Debug, Clone, Copy)]
pub struct ElfFile<'a> {
    pub(crate) data: &'a [u8],
    header: Elf64Header,
}

/// A loadable segment extracted from an ELF64 file.
#[derive(Debug)]
pub struct LoadSegment<'a> {
    /// Virtual address where this segment should be mapped.
    pub vaddr: u64,
    /// File content of this segment (may be shorter than `memsz`; remainder is zero-filled).
    pub data: &'a [u8],
    /// Total size of the segment in memory.
    pub memsz: u64,
    /// Segment permission flags (`PF_R = 4`, `PF_W = 2`, `PF_X = 1`).
    pub flags: u32,
}

impl<'a> ElfFile<'a> {
    /// Parse an ELF64 file from raw bytes.
    ///
    /// This validates the file header and ensures the program header table
    /// is within bounds.
    ///
    /// # Errors
    ///
    /// Returns [`ElfError`] if the header is invalid or the data is too short.
    pub fn parse(data: &'a [u8]) -> Result<Self, ElfError> {
        let header = Elf64Header::parse(data)?;
        Ok(Self { data, header })
    }

    /// Returns the virtual address of the entry point.
    #[must_use]
    pub fn entry_point(&self) -> u64 {
        self.header.e_entry
    }

    /// Returns the parsed ELF64 file header.
    #[must_use]
    pub fn header(&self) -> &Elf64Header {
        &self.header
    }

    /// Returns an iterator over `PT_LOAD` segments.
    ///
    /// Each yielded [`LoadSegment`] contains a slice into the original data
    /// for the file-backed portion and the total memory size (which may be
    /// larger if the segment has a `.bss`-like zero-fill region).
    /// The header is already validated to ensure program header offsets fit in the
    /// file data, so truncation from `u64` to `usize` is safe on 64-bit targets
    /// (and would have been caught by `InvalidOffset` on 32-bit).
    #[expect(
        clippy::cast_possible_truncation,
        reason = "ELF segment fields fit in target width"
    )]
    pub fn load_segments(&self) -> impl Iterator<Item = LoadSegment<'a>> {
        let data = self.data;
        let phoff = self.header.e_phoff as usize;
        let phentsize = self.header.e_phentsize as usize;
        let phnum = self.header.e_phnum as usize;

        (0..phnum).filter_map(move |i| {
            let offset = phoff + i * phentsize;
            if offset + ELF64_PHDR_SIZE > data.len() {
                return None;
            }

            let phdr = Elf64ProgramHeader::parse(data, offset);
            if phdr.seg_type != PT_LOAD {
                return None;
            }

            let file_offset = phdr.offset as usize;
            let file_size = phdr.filesz as usize;

            // Bounds-check the segment data within the file
            let seg_data = if file_size == 0 {
                &[] as &[u8]
            } else if file_offset + file_size <= data.len() {
                &data[file_offset..file_offset + file_size]
            } else {
                // Truncated segment — return what we can
                &data[file_offset.min(data.len())..data.len()]
            };

            Some(LoadSegment {
                vaddr: phdr.vaddr,
                data: seg_data,
                memsz: phdr.memsz,
                flags: phdr.flags,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests::{append_phdr, make_elf_header};

    /// Build a minimal ELF with one PT_LOAD segment containing `payload`.
    fn make_elf_with_load_segment(payload: &[u8]) -> Vec<u8> {
        let mut buf = make_elf_header();

        // Segment data will be appended after header + 1 phdr
        let data_offset = 64 + 56; // ehdr + 1 phdr
        let pf_r_x: u32 = 4 | 1; // PF_R | PF_X

        append_phdr(
            &mut buf,
            1, // PT_LOAD
            pf_r_x,
            data_offset as u64,
            0x0040_0000,
            payload.len() as u64,
            payload.len() as u64 + 0x100, // memsz > filesz (BSS region)
        );

        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn parse_valid_elf_file() {
        let buf = make_elf_header();
        let elf = ElfFile::parse(&buf).expect("valid ELF");
        assert_eq!(elf.entry_point(), 0x0040_1000);
    }

    #[test]
    fn entry_point_matches_header() {
        let mut buf = make_elf_header();
        buf[24..32].copy_from_slice(&0xDEAD_BEEFu64.to_le_bytes());
        let elf = ElfFile::parse(&buf).expect("valid ELF");
        assert_eq!(elf.entry_point(), 0xDEAD_BEEF);
    }

    #[test]
    fn no_segments_yields_empty_iterator() {
        let buf = make_elf_header();
        let elf = ElfFile::parse(&buf).expect("valid ELF");
        assert_eq!(elf.load_segments().count(), 0);
    }

    #[test]
    fn one_load_segment() {
        let payload = b"hello, elf!";
        let buf = make_elf_with_load_segment(payload);

        let elf = ElfFile::parse(&buf).expect("valid ELF");
        let segments: Vec<_> = elf.load_segments().collect();

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].vaddr, 0x0040_0000);
        assert_eq!(segments[0].data, payload);
        assert_eq!(segments[0].memsz, payload.len() as u64 + 0x100);
        assert_eq!(segments[0].flags, 4 | 1); // PF_R | PF_X
    }

    #[test]
    fn multiple_segments_filters_non_load() {
        let mut buf = make_elf_header();

        let pf_r: u32 = 4;
        let pf_rw: u32 = 4 | 2;
        let pt_note: u32 = 4;

        // PT_LOAD segment
        let data_offset = 64 + 56 * 3; // after 3 phdrs
        append_phdr(&mut buf, 1, pf_r, data_offset as u64, 0x40_0000, 4, 4);

        // PT_NOTE segment (should be skipped)
        append_phdr(&mut buf, pt_note, 0, 0, 0, 0, 0);

        // Another PT_LOAD segment
        append_phdr(
            &mut buf,
            1,
            pf_rw,
            (data_offset + 4) as u64,
            0x60_0000,
            4,
            0x1000,
        );

        // Append segment data
        buf.extend_from_slice(&[0xAA; 4]); // first segment data
        buf.extend_from_slice(&[0xBB; 4]); // second segment data

        let elf = ElfFile::parse(&buf).expect("valid ELF");
        let segments: Vec<_> = elf.load_segments().collect();

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].vaddr, 0x40_0000);
        assert_eq!(segments[0].data, &[0xAA; 4]);
        assert_eq!(segments[1].vaddr, 0x60_0000);
        assert_eq!(segments[1].data, &[0xBB; 4]);
        assert_eq!(segments[1].memsz, 0x1000);
    }

    #[test]
    fn bss_segment_with_zero_filesz() {
        let mut buf = make_elf_header();

        // PT_LOAD with filesz=0 (pure BSS)
        append_phdr(&mut buf, 1, 4 | 2, 0, 0x60_0000, 0, 0x4000);

        let elf = ElfFile::parse(&buf).expect("valid ELF");
        let segments: Vec<_> = elf.load_segments().collect();

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].data.len(), 0);
        assert_eq!(segments[0].memsz, 0x4000);
    }

    #[test]
    fn header_accessor() {
        let buf = make_elf_header();
        let elf = ElfFile::parse(&buf).expect("valid ELF");
        assert_eq!(elf.header().e_machine, 62);
    }

    #[test]
    fn parse_rejects_invalid_data() {
        assert!(ElfFile::parse(&[]).is_err());
        assert!(ElfFile::parse(&[0u8; 32]).is_err());
    }
}
