//! Initrd (initial ramdisk) CPIO archive creation.
//!
//! Compiles userspace binaries and packages them into a CPIO newc archive.

use anyhow::{Context, Result, bail};
use hadris_cpio::write::file_tree::{FileNode, FileTree};
use hadris_cpio::write::{CpioWriteOptions, CpioWriter};
use std::path::PathBuf;

use crate::cache::CacheManifest;
use crate::compile::{self, ArtifactMap, CompileMode};
use crate::config::ResolvedConfig;
use crate::crate_graph::{self, CrateContext};
use crate::sysroot;

/// Builds all userspace binaries and packages them into `build/initrd.cpio`.
///
/// Accepts pre-compiled host artifacts to avoid recompiling proc-macros
/// (which would invalidate the host crate cache by changing artifact mtimes).
/// Also accepts the cache manifest for userspace sysroot caching.
pub fn build_initrd(
    config: &ResolvedConfig,
    host_artifacts: &ArtifactMap,
    cache: &mut CacheManifest,
    force: bool,
) -> Result<PathBuf> {
    let output_path = config.root.join("build/initrd.cpio");

    // Step 1: Build userspace sysroot (with cache check).
    let user_target_name = "x86_64-unknown-hadron-user";
    let user_target_spec = config.root.join(format!("targets/{user_target_name}.json"));
    if !user_target_spec.exists() {
        bail!(
            "userspace target spec not found: {}",
            user_target_spec.display()
        );
    }

    let user_sysroot_dir = if !force
        && cache
            .is_sysroot_fresh(user_target_name, 2)
            .is_fresh()
    {
        println!("  Userspace sysroot unchanged, skipping.");
        sysroot::sysroot_output_paths(&config.root, user_target_name).sysroot_dir
    } else {
        println!("  Building userspace sysroot for {user_target_name}...");
        let user_sysroot = sysroot::build_sysroot(
            &config.root,
            &user_target_spec,
            user_target_name,
            2, // always optimize userspace
        )?;
        cache.record_sysroot(
            user_target_name,
            2,
            user_sysroot.core_rlib,
            user_sysroot.compiler_builtins_rlib,
            user_sysroot.alloc_rlib,
        );
        user_sysroot.sysroot_dir
    };

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

    // Use the pre-compiled host artifacts (proc-macros) passed from the main
    // pipeline instead of recompiling them. This is critical: recompiling host
    // crates here would overwrite their artifacts and invalidate the cache.
    let mut artifacts = ArtifactMap::default();
    let host_crates =
        crate_graph::resolve_and_sort(&registry, &config.root, &sysroot_src, &CrateContext::Host)?;
    for krate in &host_crates {
        if let Some(path) = host_artifacts.get(&krate.name) {
            artifacts.insert(&krate.name, path.to_path_buf());
        }
    }

    // Compile userspace crates.
    let mut bin_artifacts = Vec::new();
    for krate in &user_crates {
        println!("  Compiling {} (userspace)...", krate.name);
        let artifact = compile::compile_crate(
            krate,
            config,
            user_target_str,
            &user_sysroot_dir,
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
