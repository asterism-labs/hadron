//! Initrd (initial ramdisk) CPIO archive creation.
//!
//! Compiles userspace binaries and packages them into a CPIO newc archive.

use anyhow::{Context, Result, bail};
use hadris_cpio::write::file_tree::{FileNode, FileTree};
use hadris_cpio::write::{CpioWriteOptions, CpioWriter};
use std::path::PathBuf;

use crate::compile::{self, ArtifactMap, CompileMode};
use crate::config::ResolvedConfig;
use crate::crate_graph::{self, CrateContext};
use crate::sysroot;

/// Builds all userspace binaries and packages them into `build/initrd.cpio`.
pub fn build_initrd(config: &ResolvedConfig) -> Result<PathBuf> {
    let output_path = config.root.join("build/initrd.cpio");

    // Step 1: Build userspace sysroot (core + compiler_builtins only, no alloc).
    let user_target_name = "x86_64-unknown-hadron-user";
    let user_target_spec = config.root.join(format!("targets/{user_target_name}.json"));
    if !user_target_spec.exists() {
        bail!(
            "userspace target spec not found: {}",
            user_target_spec.display()
        );
    }

    println!("  Building userspace sysroot for {user_target_name}...");
    let user_sysroot = sysroot::build_sysroot(
        &config.root,
        &user_target_spec,
        user_target_name,
        2, // always optimize userspace
    )?;

    // Step 2: Compile userspace crates.
    let registry = crate_graph::load_crate_registry(&config.root)?;
    let sysroot_src = sysroot::sysroot_src_dir()?;

    let user_crates = crate_graph::resolve_and_sort(
        &registry,
        &config.root,
        &sysroot_src,
        &CrateContext::Userspace,
    )?;

    let user_target_str = user_target_spec
        .to_str()
        .expect("target spec path is valid UTF-8");

    // Host artifacts are needed for proc-macros used by userspace crates.
    let mut artifacts = ArtifactMap::default();

    // First compile any host crates needed (proc-macros).
    let host_crates =
        crate_graph::resolve_and_sort(&registry, &config.root, &sysroot_src, &CrateContext::Host)?;
    for krate in &host_crates {
        if artifacts.get(&krate.name).is_none() {
            let artifact = compile::compile_host_crate(krate, &config.root, &artifacts)?;
            artifacts.insert(&krate.name, artifact);
        }
    }

    // Then compile userspace crates.
    let mut bin_artifacts = Vec::new();
    for krate in &user_crates {
        println!("  Compiling {} (userspace)...", krate.name);
        let artifact = compile::compile_crate(
            krate,
            config,
            user_target_str,
            &user_sysroot.sysroot_dir,
            &artifacts,
            None, // no hadron_config for userspace
            Some(user_target_name), // separate output dir for userspace
            None, // no linker script for userspace
            CompileMode::Build,
        )?;
        if krate.crate_type == "bin" {
            bin_artifacts.push((krate.name.clone(), artifact.clone()));
        }
        artifacts.insert(&krate.name, artifact);
    }

    // Step 3: Package into CPIO.
    let mut tree = FileTree::new();
    for (name, bin_path) in &bin_artifacts {
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
