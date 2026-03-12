//! CPIO newc format parser.
//!
//! Provides a zero-copy iterator over entries in a CPIO archive using the
//! "newc" (SVR4 with CRC) format. Each entry has a 110-byte ASCII hex
//! header followed by filename and file data, each padded to 4-byte
//! boundaries. The archive ends with a `TRAILER!!!` sentinel entry.

#![no_std]

/// Magic bytes for the CPIO newc format.
const NEWC_MAGIC: &[u8; 6] = b"070701";

/// Size of the newc header in bytes.
const HEADER_SIZE: usize = 110;

/// A single entry in a CPIO archive.
#[derive(Debug)]
pub struct CpioEntry<'a> {
    /// File path (without leading `./`).
    pub name: &'a str,
    /// File mode (permissions + type bits).
    pub mode: u32,
    /// File data.
    pub data: &'a [u8],
}

impl<'a> CpioEntry<'a> {
    /// Returns `true` if this entry is a directory (mode & 0o170000 == 0o040000).
    #[must_use]
    pub const fn is_directory(&self) -> bool {
        self.mode & 0o170_000 == 0o040_000
    }

    /// Returns `true` if this entry is a regular file (mode & 0o170000 == 0o100000).
    #[must_use]
    pub const fn is_file(&self) -> bool {
        self.mode & 0o170_000 == 0o100_000
    }

    /// Returns `true` if this entry is a symbolic link (mode & 0o120000).
    #[must_use]
    pub const fn is_symlink(&self) -> bool {
        self.mode & 0o170_000 == 0o120_000
    }
}

/// A CPIO newc archive backed by a byte slice.
#[derive(Debug)]
pub struct CpioArchive<'a> {
    data: &'a [u8],
}

impl<'a> CpioArchive<'a> {
    /// Create a new archive view over the given data.
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Return an iterator over the archive entries.
    #[must_use]
    pub const fn entries(&self) -> CpioIter<'a> {
        CpioIter {
            data: self.data,
            offset: 0,
        }
    }
}

/// Iterator over CPIO archive entries.
#[derive(Debug)]
pub struct CpioIter<'a> {
    data: &'a [u8],
    offset: usize,
}

/// Round `val` up to the next multiple of 4.
const fn align4(val: usize) -> usize {
    (val + 3) & !3
}

/// Parse an 8-character ASCII hex field from the header.
fn parse_hex8(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 8 {
        return None;
    }
    let mut val = 0u32;
    let mut i = 0;
    while i < 8 {
        let digit = match bytes[i] {
            b'0'..=b'9' => bytes[i] - b'0',
            b'a'..=b'f' => bytes[i] - b'a' + 10,
            b'A'..=b'F' => bytes[i] - b'A' + 10,
            _ => return None,
        };
        val = val << 4 | u32::from(digit);
        i += 1;
    }
    Some(val)
}

impl<'a> Iterator for CpioIter<'a> {
    type Item = CpioEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let remaining = self.data.len().checked_sub(self.offset)?;
        if remaining < HEADER_SIZE {
            return None;
        }

        let hdr = &self.data[self.offset..self.offset + HEADER_SIZE];

        // Verify magic.
        if &hdr[0..6] != NEWC_MAGIC {
            return None;
        }

        // Parse fields from the fixed header layout:
        //   0..6    magic
        //   6..14   ino
        //  14..22   mode
        //  22..30   uid
        //  30..38   gid
        //  38..46   nlink
        //  46..54   mtime
        //  54..62   filesize
        //  62..70   devmajor
        //  70..78   devminor
        //  78..86   rdevmajor
        //  86..94   rdevminor
        //  94..102  namesize
        // 102..110  check
        let mode = parse_hex8(&hdr[14..22])?;
        let filesize = parse_hex8(&hdr[54..62])? as usize;
        let namesize = parse_hex8(&hdr[94..102])? as usize;

        // Name starts right after the header, padded to 4 bytes.
        let name_start = self.offset + HEADER_SIZE;
        let name_end = name_start + namesize;
        if name_end > self.data.len() {
            return None;
        }

        // Name includes a trailing NUL — strip it.
        let name_bytes = &self.data[name_start..name_end.saturating_sub(1)];
        let name = core::str::from_utf8(name_bytes).ok()?;

        // Check for trailer.
        if name == "TRAILER!!!" {
            return None;
        }

        // Data starts after the name, padded to 4 bytes.
        let data_start = align4(name_end);
        let data_end = data_start + filesize;
        if data_end > self.data.len() {
            return None;
        }

        let data = &self.data[data_start..data_end];

        // Advance to the next entry (data is also padded to 4 bytes).
        self.offset = align4(data_end);

        // Strip leading "./" from names if present.
        let name = name.strip_prefix("./").unwrap_or(name);

        Some(CpioEntry { name, mode, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal CPIO newc archive in memory for testing.
    fn build_test_archive(entries: &[(&str, u32, &[u8])]) -> alloc::vec::Vec<u8> {
        extern crate alloc;
        use alloc::vec::Vec;

        let mut buf = Vec::new();

        for &(name, mode, data) in entries {
            let name_with_nul = name.len() + 1;
            // Write header (110 bytes of ASCII hex).
            let hdr = alloc::format!(
                "070701\
                 00000000\
                 {:08x}\
                 00000000\
                 00000000\
                 00000001\
                 00000000\
                 {:08x}\
                 00000000\
                 00000000\
                 00000000\
                 00000000\
                 {:08x}\
                 00000000",
                mode,
                data.len(),
                name_with_nul,
            );
            assert_eq!(hdr.len(), HEADER_SIZE, "bad header length");
            buf.extend_from_slice(hdr.as_bytes());

            // Write name + NUL + padding.
            buf.extend_from_slice(name.as_bytes());
            buf.push(0);
            while (buf.len()) % 4 != 0 {
                buf.push(0);
            }

            // Write data + padding.
            buf.extend_from_slice(data);
            while buf.len() % 4 != 0 {
                buf.push(0);
            }
        }

        // Write trailer entry.
        let trailer_name = "TRAILER!!!";
        let name_with_nul = trailer_name.len() + 1;
        let hdr = alloc::format!(
            "070701\
             00000000\
             00000000\
             00000000\
             00000000\
             00000001\
             00000000\
             00000000\
             00000000\
             00000000\
             00000000\
             00000000\
             {:08x}\
             00000000",
            name_with_nul,
        );
        buf.extend_from_slice(hdr.as_bytes());
        buf.extend_from_slice(trailer_name.as_bytes());
        buf.push(0);
        while buf.len() % 4 != 0 {
            buf.push(0);
        }

        buf
    }

    #[test]
    fn parse_single_file() {
        let archive = build_test_archive(&[("hello.txt", 0o100644, b"Hello, world!")]);
        let cpio = CpioArchive::new(&archive);
        let entries: alloc::vec::Vec<_> = cpio.entries().collect();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "hello.txt");
        assert_eq!(entries[0].mode, 0o100644);
        assert_eq!(entries[0].data, b"Hello, world!");
        assert!(entries[0].is_file());
        assert!(!entries[0].is_directory());
    }

    #[test]
    fn parse_directory_and_files() {
        let archive = build_test_archive(&[
            ("bin", 0o040755, b""),
            ("bin/init", 0o100755, b"\x7fELF"),
            ("lib", 0o040755, b""),
        ]);
        let cpio = CpioArchive::new(&archive);
        let entries: alloc::vec::Vec<_> = cpio.entries().collect();

        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_directory());
        assert_eq!(entries[0].name, "bin");
        assert!(entries[1].is_file());
        assert_eq!(entries[1].name, "bin/init");
        assert_eq!(entries[1].data, b"\x7fELF");
        assert!(entries[2].is_directory());
    }

    #[test]
    fn empty_archive() {
        let archive = build_test_archive(&[]);
        let cpio = CpioArchive::new(&archive);
        let entries: alloc::vec::Vec<_> = cpio.entries().collect();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn strips_dot_slash_prefix() {
        let archive = build_test_archive(&[("./foo/bar", 0o100644, b"data")]);
        let cpio = CpioArchive::new(&archive);
        let entries: alloc::vec::Vec<_> = cpio.entries().collect();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "foo/bar");
    }

    #[test]
    fn symlink_detection() {
        let archive = build_test_archive(&[("link", 0o120777, b"/target")]);
        let cpio = CpioArchive::new(&archive);
        let entries: alloc::vec::Vec<_> = cpio.entries().collect();

        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_symlink());
        assert!(!entries[0].is_file());
        assert!(!entries[0].is_directory());
        assert_eq!(entries[0].data, b"/target");
    }

    #[test]
    fn truncated_archive_returns_none() {
        let archive = build_test_archive(&[("test", 0o100644, b"data")]);
        // Truncate to partial header.
        let truncated = &archive[..50];
        let cpio = CpioArchive::new(truncated);
        let entries: alloc::vec::Vec<_> = cpio.entries().collect();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn bad_magic_returns_none() {
        let mut archive = build_test_archive(&[("test", 0o100644, b"data")]);
        archive[0] = b'X';
        let cpio = CpioArchive::new(&archive);
        let entries: alloc::vec::Vec<_> = cpio.entries().collect();
        assert_eq!(entries.len(), 0);
    }
}

extern crate alloc;
