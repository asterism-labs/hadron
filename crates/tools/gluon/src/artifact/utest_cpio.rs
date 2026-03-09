//! Minimal CPIO archive for userspace test binaries.
//!
//! Packages a single userspace test binary as `/bin/init` in a CPIO newc
//! archive. The kernel boots PID 1 from `/bin/init`, and in `--utest` mode
//! its exit code is forwarded to `isa-debug-exit`.

use anyhow::{Context, Result};
use hadris_cpio::write::file_tree::{FileNode, FileTree};
use hadris_cpio::write::{CpioWriteOptions, CpioWriter};
use std::path::{Path, PathBuf};

/// Package a compiled userspace test binary into a minimal CPIO archive.
///
/// Layout:
/// ```text
/// /bin/init     (the test binary, mode 0o755)
/// /etc/profile  (PATH=/bin\nHOME=/\n)
/// /tmp/         (empty directory)
/// ```
///
/// The archive is written to `build/utests/<test_name>.cpio`.
pub fn build_utest_cpio(root: &Path, test_name: &str, binary_path: &Path) -> Result<PathBuf> {
    let output_path = root.join("build/utests").join(format!("{test_name}.cpio"));

    let data = std::fs::read(binary_path)
        .with_context(|| format!("reading utest binary: {}", binary_path.display()))?;

    let mut tree = FileTree::new();

    // /bin/init — the test binary.
    println!("  Utest CPIO: /bin/init ({} bytes)", data.len());
    tree.add(FileNode::dir(
        "bin",
        vec![FileNode::file("init", data, 0o755)],
        0o755,
    ));

    // /etc/profile — minimal environment.
    let profile_contents = b"PATH=/bin\nHOME=/\n".to_vec();
    tree.add(FileNode::dir(
        "etc",
        vec![FileNode::file("profile", profile_contents, 0o644)],
        0o755,
    ));

    // /tmp/ — empty scratch directory.
    tree.add(FileNode::dir("tmp", vec![], 0o1777));

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(&output_path)
        .with_context(|| format!("creating {}", output_path.display()))?;

    let writer = CpioWriter::new(CpioWriteOptions::default());
    writer
        .write(&mut file, &tree)
        .context("writing utest CPIO archive")?;

    println!("  Utest CPIO: {}", output_path.display());
    Ok(output_path)
}
