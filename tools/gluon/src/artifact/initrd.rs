//! Initrd (initial ramdisk) CPIO archive creation.
//!
//! Packages pre-compiled userspace binaries into a CPIO newc archive
//! with a Unix-like directory layout (`/bin`, `/etc`, `/tmp`), and
//! creates symlinks for coreutils multi-call dispatch.

use anyhow::{Context, Result};
use hadris_cpio::write::file_tree::{FileNode, FileTree};
use hadris_cpio::write::{CpioWriteOptions, CpioWriter};
use std::path::PathBuf;

use crate::config::ResolvedConfig;

/// Coreutils commands that get symlinks pointing to `/bin/coreutils`.
const COREUTILS_COMMANDS: &[&str] = &[
    "echo", "cat", "ls", "uname", "uptime", "clear", "true", "false", "yes", "env", "pwd",
];

/// Mapping from lepton crate name to binary name in `/bin/`.
fn binary_name(crate_name: &str) -> &str {
    match crate_name {
        "lepton-init" => "init",
        "lepton-shell" => "sh",
        "lepton-coreutils" => "coreutils",
        other => other.strip_prefix("lepton-").unwrap_or(other),
    }
}

/// Packages already-compiled userspace binaries into `build/initrd.cpio`.
///
/// Creates a Unix-like layout:
/// ```text
/// /bin/init          (lepton-init)
/// /bin/sh            (lepton-shell)
/// /bin/coreutils     (lepton-coreutils)
/// /bin/echo          → /bin/coreutils (symlink)
/// /bin/cat           → /bin/coreutils (symlink)
/// /bin/...           → /bin/coreutils (symlink)
/// /etc/profile       (PATH=/bin\nHOME=/\n)
/// /tmp/              (empty directory)
/// ```
pub fn build_initrd(
    config: &ResolvedConfig,
    bin_artifacts: &[(String, PathBuf)],
) -> Result<PathBuf> {
    let output_path = config.root.join("build/initrd.cpio");

    let mut tree = FileTree::new();

    // Build /bin/ directory contents.
    let mut bin_children: Vec<FileNode> = Vec::new();

    for (name, bin_path) in bin_artifacts {
        let data = std::fs::read(bin_path)
            .with_context(|| format!("reading userspace binary: {}", bin_path.display()))?;

        let bin_name = binary_name(name);
        println!("  Initrd: /bin/{bin_name} ({} bytes)", data.len());
        bin_children.push(FileNode::file(bin_name, data, 0o755));
    }

    // Create symlinks for coreutils multi-call commands inside /bin/.
    for cmd in COREUTILS_COMMANDS {
        println!("  Initrd: /bin/{cmd} -> /bin/coreutils (symlink)");
        bin_children.push(FileNode::symlink(cmd, "/bin/coreutils"));
    }

    tree.add(FileNode::dir("bin", bin_children, 0o755));

    // Create /etc/profile with default environment.
    let profile_contents = b"PATH=/bin\nHOME=/\n".to_vec();
    println!("  Initrd: /etc/profile ({} bytes)", profile_contents.len());
    tree.add(FileNode::dir(
        "etc",
        vec![FileNode::file("profile", profile_contents, 0o644)],
        0o755,
    ));

    // Create /tmp/ (empty directory).
    println!("  Initrd: /tmp/ (empty directory)");
    tree.add(FileNode::dir("tmp", vec![], 0o1777));

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(&output_path)
        .with_context(|| format!("creating {}", output_path.display()))?;

    let writer = CpioWriter::new(CpioWriteOptions::default());
    writer
        .write(&mut file, &tree)
        .context("writing CPIO archive")?;

    println!("  Initrd: {}", output_path.display());
    Ok(output_path)
}
