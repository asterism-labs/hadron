//! Initrd (initial ramdisk) CPIO archive creation.
//!
//! Builds the Rust userspace init binary for the `x86_64-unknown-hadron-user`
//! target, then packages it as `/init` in a CPIO newc archive at
//! `target/initrd.cpio`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use hadris_cpio::write::file_tree::{FileNode, FileTree};
use hadris_cpio::write::{CpioWriteOptions, CpioWriter};

/// Builds the userspace init binary and packages it into a CPIO initrd.
///
/// 1. Invokes `cargo build` on `userspace/init/` for the userspace target.
/// 2. Reads the resulting ELF binary.
/// 3. Creates a CPIO archive with the binary as `/init`.
pub fn build_initrd(workspace_root: &Path) -> Result<PathBuf> {
    let init_elf = build_userspace_init(workspace_root)?;

    let init_data = std::fs::read(&init_elf)
        .with_context(|| format!("failed to read init binary: {}", init_elf.display()))?;
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

/// Cross-compile the userspace init binary and return the path to the ELF.
fn build_userspace_init(workspace_root: &Path) -> Result<PathBuf> {
    let target_spec = workspace_root
        .join("targets/x86_64-unknown-hadron-user.json")
        .canonicalize()
        .context("userspace target spec not found")?;
    let target_spec_str = target_spec
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("target spec path is not valid UTF-8"))?;

    let init_dir = workspace_root.join("userspace/init");
    if !init_dir.exists() {
        anyhow::bail!("Userspace init crate not found at: {}", init_dir.display());
    }

    let sh = xshell::Shell::new()?;
    sh.change_dir(&init_dir);

    println!("Building userspace init binary...");
    xshell::cmd!(
        sh,
        "cargo build
            --target {target_spec_str}
            -Zjson-target-spec
            -Zbuild-std=core,compiler_builtins
            -Zbuild-std-features=compiler-builtins-mem
            --release"
    )
    .run()
    .context("failed to build userspace init binary")?;

    // The output binary is in the init crate's own target directory.
    let elf_path = init_dir.join("target/x86_64-unknown-hadron-user/release/hadron-init");
    if !elf_path.exists() {
        anyhow::bail!(
            "Init binary not found after build at: {}",
            elf_path.display()
        );
    }

    Ok(elf_path)
}
