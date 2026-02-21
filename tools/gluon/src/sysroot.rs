//! Sysroot compilation for custom targets.
//!
//! Compiles `core`, `compiler_builtins`, and `alloc` from the rustc sysroot
//! source into `build/sysroot/lib/rustlib/<target>/lib/`. Downstream crates
//! use `--sysroot build/sysroot` to link against these.

use crate::rustc_cmd::RustcCommandBuilder;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};


/// Paths to the compiled sysroot rlibs.
pub struct SysrootOutput {
    pub sysroot_dir: PathBuf,
    #[allow(dead_code)] // available for direct rlib access
    pub lib_dir: PathBuf,
    pub core_rlib: PathBuf,
    pub compiler_builtins_rlib: PathBuf,
    pub alloc_rlib: PathBuf,
}

/// Locate the rustc sysroot source directory.
pub fn sysroot_src_dir() -> Result<PathBuf> {
    let sysroot = crate::rustc_info::sysroot_path();

    let src_dir = sysroot.join("lib/rustlib/src/rust/library");
    if !src_dir.exists() {
        bail!(
            "sysroot source not found at {}\n\
             Install with: rustup component add rust-src",
            src_dir.display()
        );
    }

    Ok(src_dir)
}

/// Detect the edition from a crate's Cargo.toml.
///
/// Uses simple string matching rather than a full TOML parser since we only
/// need the `edition` field from `[package]`.
fn detect_edition(crate_dir: &Path) -> Result<String> {
    let cargo_toml = crate_dir.join("Cargo.toml");
    if cargo_toml.exists() {
        let contents = std::fs::read_to_string(&cargo_toml)?;
        for line in contents.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("edition") {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('=') {
                    let rest = rest.trim();
                    let rest = rest.trim_matches('"');
                    if !rest.is_empty() {
                        return Ok(rest.to_string());
                    }
                }
            }
        }
    }
    // Fall back to 2021 if we can't detect.
    Ok("2021".into())
}

/// Compute sysroot output paths without compiling.
///
/// Used by the cache layer to check if the sysroot rlibs still exist.
pub fn sysroot_output_paths(project_root: &Path, target_name: &str) -> SysrootOutput {
    let sysroot_dir = project_root.join("build/sysroot");
    let lib_dir = sysroot_dir
        .join("lib/rustlib")
        .join(target_name)
        .join("lib");

    SysrootOutput {
        core_rlib: lib_dir.join("libcore.rlib"),
        compiler_builtins_rlib: lib_dir.join("libcompiler_builtins.rlib"),
        alloc_rlib: lib_dir.join("liballoc.rlib"),
        sysroot_dir,
        lib_dir,
    }
}

/// Compile the sysroot crates (core, compiler_builtins, alloc) for the given target.
pub fn build_sysroot(
    project_root: &Path,
    target_spec: &Path,
    target_name: &str,
    opt_level: u32,
) -> Result<SysrootOutput> {
    let sysroot_src = sysroot_src_dir()?;

    let sysroot_dir = project_root.join("build/sysroot");
    let lib_dir = sysroot_dir
        .join("lib/rustlib")
        .join(target_name)
        .join("lib");
    std::fs::create_dir_all(&lib_dir)
        .context("failed to create sysroot output directory")?;

    let target_flag = target_spec
        .to_str()
        .context("non-UTF-8 target spec path")?;

    // Step 1: Compile core.
    let core_edition = detect_edition(&sysroot_src.join("core"))?;
    println!("  Compiling core (edition {core_edition})...");
    let core_rlib = compile_sysroot_crate(
        "core",
        &sysroot_src.join("core/src/lib.rs"),
        &core_edition,
        &lib_dir,
        target_flag,
        opt_level,
        &[],
        &["no_fp_fmt_parse"],
        &[],
    )?;

    // Step 2: Compile compiler_builtins.
    let cb_edition =
        detect_edition(&sysroot_src.join("compiler-builtins/compiler-builtins"))?;
    println!("  Compiling compiler_builtins (edition {cb_edition})...");
    let compiler_builtins_rlib = compile_sysroot_crate(
        "compiler_builtins",
        &sysroot_src.join("compiler-builtins/compiler-builtins/src/lib.rs"),
        &cb_edition,
        &lib_dir,
        target_flag,
        opt_level,
        &[("core", &core_rlib)],
        &[],
        &["compiler-builtins", "mem", "rustc-dep-of-std"],
    )?;

    // Step 3: Compile alloc.
    let alloc_edition = detect_edition(&sysroot_src.join("alloc"))?;
    println!("  Compiling alloc (edition {alloc_edition})...");
    let alloc_rlib = compile_sysroot_crate(
        "alloc",
        &sysroot_src.join("alloc/src/lib.rs"),
        &alloc_edition,
        &lib_dir,
        target_flag,
        opt_level,
        &[
            ("core", &core_rlib),
            ("compiler_builtins", &compiler_builtins_rlib),
        ],
        &["no_fp_fmt_parse"],
        &[],
    )?;

    Ok(SysrootOutput {
        sysroot_dir,
        lib_dir,
        core_rlib,
        compiler_builtins_rlib,
        alloc_rlib,
    })
}

/// Compile a single sysroot crate with rustc.
fn compile_sysroot_crate(
    crate_name: &str,
    source: &Path,
    edition: &str,
    out_dir: &Path,
    target: &str,
    opt_level: u32,
    externs: &[(&str, &Path)],
    cfgs: &[&str],
    features: &[&str],
) -> Result<PathBuf> {
    let mut cmd = RustcCommandBuilder::new("rustc");
    cmd.crate_name(crate_name)
        .edition(edition)
        .crate_type("rlib")
        .unstable_options()
        .arg("-Zforce-unstable-if-unmarked")
        .allow("internal_features")
        .panic_abort()
        .opt_level(opt_level)
        .target(target)
        .out_dir(out_dir)
        .emit("metadata,link");

    for (name, path) in externs {
        cmd.add_extern(name, path);
    }

    for flag in cfgs {
        cmd.cfg(flag);
    }

    for feat in features {
        cmd.feature(feat);
    }

    cmd.source(source);

    cmd.run_checked("compile")?;

    let rlib = out_dir.join(format!("lib{crate_name}.rlib"));
    if !rlib.exists() {
        bail!(
            "expected rlib not found after compilation: {}",
            rlib.display()
        );
    }

    Ok(rlib)
}
