//! Initrd (initial ramdisk) CPIO archive creation.
//!
//! Builds a CPIO newc archive containing `/init` from the userspace ELF binary,
//! written to `target/initrd.cpio`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use hadris_cpio::write::file_tree::{FileNode, FileTree};
use hadris_cpio::write::{CpioWriteOptions, CpioWriter};

/// Builds the initrd CPIO archive and returns the output path.
///
/// Reads the init binary from `userspace/test_init.elf` and packages it as
/// `/init` (mode 0o755) in a CPIO newc archive at `target/initrd.cpio`.
pub fn build_initrd(workspace_root: &Path) -> Result<PathBuf> {
    let init_binary = workspace_root.join("userspace/test_init.elf");
    if !init_binary.exists() {
        anyhow::bail!(
            "Init binary not found at: {}",
            init_binary.display()
        );
    }

    let init_data = std::fs::read(&init_binary)
        .with_context(|| format!("failed to read init binary: {}", init_binary.display()))?;
    let init_size = init_data.len();

    let mut tree = FileTree::new();
    tree.add(FileNode::file("init", init_data, 0o755));

    let output_path = workspace_root.join("target/initrd.cpio");
    std::fs::create_dir_all(output_path.parent().unwrap())?;

    let mut file = std::fs::File::create(&output_path)
        .with_context(|| format!("failed to create {}", output_path.display()))?;

    let writer = CpioWriter::new(CpioWriteOptions::default());
    writer
        .write(&mut file, &tree)
        .context("failed to write CPIO archive")?;

    println!("Initrd: {} ({} bytes)", output_path.display(), init_size);
    Ok(output_path)
}
