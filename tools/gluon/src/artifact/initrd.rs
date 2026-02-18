//! Initrd (initial ramdisk) CPIO archive creation.
//!
//! Packages pre-compiled userspace binaries into a CPIO newc archive.

use anyhow::{Context, Result};
use hadris_cpio::write::file_tree::{FileNode, FileTree};
use hadris_cpio::write::{CpioWriteOptions, CpioWriter};
use std::path::PathBuf;

use crate::config::ResolvedConfig;

/// Packages already-compiled userspace binaries into `build/initrd.cpio`.
///
/// Accepts a list of (crate_name, binary_path) pairs from the pipeline's
/// artifact map.
pub fn build_initrd(
    config: &ResolvedConfig,
    bin_artifacts: &[(String, PathBuf)],
) -> Result<PathBuf> {
    let output_path = config.root.join("build/initrd.cpio");

    let mut tree = FileTree::new();
    for (name, bin_path) in bin_artifacts {
        let data = std::fs::read(bin_path)
            .with_context(|| format!("reading userspace binary: {}", bin_path.display()))?;

        // Use binary name without crate prefix as the filename in initrd.
        let initrd_name = name.strip_prefix("hadron-").unwrap_or(name);
        println!(
            "  Initrd: /{initrd_name} ({} bytes)",
            data.len()
        );
        tree.add(FileNode::file(initrd_name, data, 0o755));
    }

    std::fs::create_dir_all(output_path.parent().unwrap())?;
    let mut file = std::fs::File::create(&output_path)
        .with_context(|| format!("creating {}", output_path.display()))?;

    let writer = CpioWriter::new(CpioWriteOptions::default());
    writer
        .write(&mut file, &tree)
        .context("writing CPIO archive")?;

    println!("  Initrd: {}", output_path.display());
    Ok(output_path)
}
