//! CPIO initramfs unpacker.
//!
//! Extracts the contents of a CPIO newc archive into the VFS. Directories
//! are created recursively and file data is written into the root filesystem.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec;

use hadris_cpio::mode::FileType;
use hadris_cpio::CpioReader;
use hadris_io::Cursor;

use super::{poll_immediate, FsError, Inode, InodeType, Permissions};

/// Unpack a CPIO newc archive into the given root inode.
///
/// Returns the number of files unpacked. Directories are created as needed;
/// the CPIO root `.` entry is skipped.
///
/// # Panics
///
/// Panics if the CPIO archive is malformed or if file/directory creation fails.
#[must_use]
pub fn unpack_cpio(initrd: &[u8], root: &Arc<dyn Inode>) -> usize {
    let mut reader = CpioReader::new(Cursor::new(initrd));
    let mut name_buf = [0u8; 512];
    let mut file_count = 0;

    loop {
        let entry = reader
            .next_entry_with_buf(&mut name_buf)
            .expect("failed to parse CPIO entry");
        let Some(entry) = entry else {
            break;
        };

        let name = entry.name_str().unwrap_or("");
        let name = name.strip_prefix('/').unwrap_or(name);

        // Skip the root directory entry and empty names.
        if name.is_empty() || name == "." {
            reader
                .skip_entry_data(&entry)
                .expect("failed to skip CPIO entry data");
            continue;
        }

        let file_type = entry.file_type();

        match file_type {
            FileType::Directory => {
                ensure_directory(root, name);
                reader
                    .skip_entry_data(&entry)
                    .expect("failed to skip CPIO directory data");
            }
            FileType::Regular => {
                let file_size = entry.file_size() as usize;

                // Ensure parent directories exist.
                if let Some(parent_path) = name.rsplit_once('/') {
                    ensure_directory(root, parent_path.0);
                }

                // Navigate to the parent directory.
                let (parent, file_name) = if let Some((dir, file)) = name.rsplit_once('/') {
                    (resolve_path(root, dir), file)
                } else {
                    (root.clone(), name)
                };

                // Create the file.
                let file_inode = parent
                    .create(file_name, InodeType::File, Permissions::all())
                    .unwrap_or_else(|e| {
                        panic!("initramfs: failed to create file '{}': {:?}", name, e)
                    });

                // Read data from CPIO and write to the inode.
                if file_size > 0 {
                    let mut buf = vec![0u8; file_size];
                    reader
                        .read_entry_data(&entry, &mut buf)
                        .expect("failed to read CPIO file data");
                    let written =
                        poll_immediate(file_inode.write(0, &buf));
                    assert_eq!(
                        written.expect("initramfs: write failed"),
                        file_size,
                        "initramfs: short write for '{name}'"
                    );
                } else {
                    reader
                        .skip_entry_data(&entry)
                        .expect("failed to skip empty CPIO file");
                }

                file_count += 1;
            }
            _ => {
                // Skip unsupported entry types (symlinks, devices, etc.).
                reader
                    .skip_entry_data(&entry)
                    .expect("failed to skip CPIO entry");
            }
        }
    }

    file_count
}

/// Ensure that a directory path exists, creating intermediate directories as needed.
fn ensure_directory(root: &Arc<dyn Inode>, path: &str) {
    let mut current = root.clone();
    for component in super::path::components(path) {
        current = match current.lookup(component) {
            Ok(inode) => inode,
            Err(FsError::NotFound) => current
                .create(component, InodeType::Directory, Permissions::all())
                .unwrap_or_else(|e| {
                    panic!(
                        "initramfs: failed to create directory '{}': {:?}",
                        component, e
                    )
                }),
            Err(e) => panic!(
                "initramfs: lookup failed for '{}': {:?}",
                component, e
            ),
        };
    }
}

/// Resolve a relative path from the given root inode.
fn resolve_path(root: &Arc<dyn Inode>, path: &str) -> Arc<dyn Inode> {
    let mut current = root.clone();
    for component in super::path::components(path) {
        current = current
            .lookup(component)
            .unwrap_or_else(|e| panic!("initramfs: resolve failed for '{}': {:?}", path, e));
    }
    current
}
