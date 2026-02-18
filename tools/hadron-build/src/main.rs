//! Hadron kernel build system.
//!
//! A standalone build tool that replaces cargo-xtask and cargo-image-runner.
//! Invokes `rustc` directly, builds a custom sysroot, and provides a
//! Kconfig-like configuration system.

mod analyzer;
mod artifact;
mod cache;
mod cli;
mod compile;
mod config;
mod crate_graph;
mod fmt;
mod run;
mod sysroot;
mod test;

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Result, bail};
use cache::{CacheManifest, CrateEntry};
use clap::Parser;
use compile::{ArtifactMap, CompileMode};
use crate_graph::CrateContext;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    match cli.command {
        cli::Command::Configure => cmd_configure(&cli),
        cli::Command::Clean => cmd_clean(),
        cli::Command::Build(ref _args) => cmd_build(&cli),
        cli::Command::Run(ref args) => cmd_run(&cli, &args.extra_args),
        cli::Command::Test(ref args) => cmd_test(&cli, args),
        cli::Command::Check => cmd_check(&cli),
        cli::Command::Clippy => cmd_clippy(&cli),
        cli::Command::Fmt(ref args) => fmt::cmd_fmt(args),
        cli::Command::Vendor => {
            todo!("vendor command")
        }
    }
}

/// Resolve configuration, print it, and generate rust-project.json.
fn cmd_configure(cli: &cli::Cli) -> Result<()> {
    let root = config::find_project_root()?;
    let resolved = config::load_config(
        &root,
        cli.profile.as_deref(),
        cli.target.as_deref(),
    )?;

    config::print_resolved(&resolved);

    // Generate rust-project.json for rust-analyzer.
    println!("\nGenerating rust-project.json...");
    analyzer::generate_rust_project(&resolved)?;

    println!("\nConfiguration resolved successfully.");
    Ok(())
}

/// Common build preparation: sysroot, host crates, config crate.
///
/// Returns all state needed to compile kernel crates in any mode.
struct BuildPrep {
    resolved: config::ResolvedConfig,
    target_spec: String,
    sysroot_dir: PathBuf,
    artifacts: ArtifactMap,
    config_rlib: PathBuf,
    kernel_crates: Vec<crate_graph::ResolvedCrate>,
    linker_script: Option<PathBuf>,
    cache: CacheManifest,
    rebuilt: HashSet<String>,
    force: bool,
}

fn prepare_build(cli: &cli::Cli) -> Result<BuildPrep> {
    let root = config::find_project_root()?;
    let resolved = config::load_config(
        &root,
        cli.profile.as_deref(),
        cli.target.as_deref(),
    )?;
    let force = cli.force;

    let target_spec_path = root.join(&resolved.target.spec);
    let target_spec = target_spec_path
        .to_str()
        .expect("target spec path is valid UTF-8")
        .to_string();

    // Load or create cache manifest.
    let rustc_hash = cache::get_rustc_version_hash()?;
    let mut cache = if force {
        CacheManifest::new(rustc_hash.clone())
    } else {
        match CacheManifest::load(&root) {
            Some(m) if m.rustc_version_hash == rustc_hash => m,
            _ => CacheManifest::new(rustc_hash.clone()),
        }
    };
    let mut rebuilt = HashSet::new();

    // Step 1: Build sysroot (with cache check).
    println!("Building sysroot for {}...", resolved.target_name);
    let sysroot_dir = if !force
        && cache
            .is_sysroot_fresh(&resolved.target_name, resolved.profile.opt_level)
            .is_fresh()
    {
        println!("  Sysroot unchanged, skipping.");
        sysroot::sysroot_output_paths(&root, &resolved.target_name).sysroot_dir
    } else {
        let sysroot_output = sysroot::build_sysroot(
            &root,
            &target_spec_path,
            &resolved.target_name,
            resolved.profile.opt_level,
        )?;
        cache.record_sysroot(
            &resolved.target_name,
            resolved.profile.opt_level,
            sysroot_output.core_rlib,
            sysroot_output.compiler_builtins_rlib,
            sysroot_output.alloc_rlib,
        );
        println!("  Sysroot ready.");
        sysroot_output.sysroot_dir
    };

    // Step 2: Load crate registry.
    let registry = crate_graph::load_crate_registry(&root)?;
    let sysroot_src = sysroot::sysroot_src_dir()?;

    // Step 3: Compile host crates (proc-macro and dependencies).
    let mut artifacts = ArtifactMap::default();

    let host_crates =
        crate_graph::resolve_and_sort(&registry, &root, &sysroot_src, &CrateContext::Host)?;

    // Quick stage check: if all host artifacts exist with matching mtimes
    // and nothing upstream was rebuilt, skip the entire stage.
    let host_names: Vec<String> = host_crates.iter().map(|k| k.name.clone()).collect();
    if !force && cache.is_stage_fresh(&host_names, &rebuilt) {
        println!("\nHost crates unchanged, skipping.");
        for krate in &host_crates {
            let artifact_path = compile::host_crate_artifact_path(krate, &root);
            artifacts.insert(&krate.name, artifact_path);
        }
    } else {
        println!("\nCompiling host crates...");
        for krate in &host_crates {
            let artifact_path = compile::host_crate_artifact_path(krate, &root);
            let dep_info_path = compile::host_crate_dep_info_path(krate, &root);
            let dep_names: Vec<String> =
                krate.deps.iter().map(|d| d.crate_name.clone()).collect();

            let flags_hash = compile::hash_args(&[
                "host".as_ref(),
                krate.name.as_ref(),
                krate.edition.as_ref(),
                krate.crate_type.as_ref(),
            ]);

            if !force {
                if let Some(entry) = cache.entries.get_mut(&krate.name) {
                    if entry.is_fresh(&flags_hash, &rebuilt, &dep_names).is_fresh() {
                        println!("  Skipping {} (host, unchanged)", krate.name);
                        artifacts.insert(&krate.name, artifact_path);
                        continue;
                    }
                }
            }

            println!("  Compiling {} (host)...", krate.name);
            let artifact = compile::compile_host_crate(krate, &root, &artifacts)?;
            if let Ok(entry) =
                CrateEntry::from_compilation(flags_hash, &artifact, &dep_info_path)
            {
                cache.entries.insert(krate.name.clone(), entry);
            }
            rebuilt.insert(krate.name.clone());
            artifacts.insert(&krate.name, artifact);
        }
    }

    // Step 4: Generate and compile hadron_config.
    println!("\nGenerating hadron_config...");
    let config_dep_info = compile::config_crate_dep_info_path(&resolved);

    // Hash the resolved config options as the flags hash for the config crate.
    let config_flags_hash = {
        let mut parts: Vec<&std::ffi::OsStr> = vec!["hadron_config".as_ref()];
        let opt_str = format!("{}", resolved.profile.opt_level);
        parts.push(opt_str.as_ref());
        parts.push(target_spec.as_ref());
        compile::hash_args(&parts)
    };

    let config_rlib_path = resolved
        .root
        .join("build/kernel")
        .join(&resolved.target_name)
        .join("debug/libhadron_config.rlib");

    let config_needs_rebuild = force || {
        match cache.entries.get_mut("hadron_config") {
            Some(entry) => !entry.is_fresh(&config_flags_hash, &rebuilt, &[]).is_fresh(),
            None => true,
        }
    };

    let config_rlib = if config_needs_rebuild {
        let rlib = compile::build_config_crate(&resolved, &target_spec, &sysroot_dir)?;
        if let Ok(entry) =
            CrateEntry::from_compilation(config_flags_hash, &rlib, &config_dep_info)
        {
            cache.entries.insert("hadron_config".into(), entry);
        }
        rebuilt.insert("hadron_config".into());
        rlib
    } else {
        println!("  Skipping hadron_config (unchanged)");
        config_rlib_path
    };
    artifacts.insert("hadron_config", config_rlib.clone());

    // Step 5: Resolve kernel crates.
    let kernel_crates = crate_graph::resolve_and_sort(
        &registry,
        &root,
        &sysroot_src,
        &CrateContext::Kernel,
    )?;

    let linker_script = resolved
        .target
        .linker_script
        .as_ref()
        .map(|ls| root.join(ls));

    Ok(BuildPrep {
        resolved,
        target_spec,
        sysroot_dir,
        artifacts,
        config_rlib,
        kernel_crates,
        linker_script,
        cache,
        rebuilt,
        force,
    })
}

/// Compile a stage of kernel crates with cache-aware freshness checks.
///
/// Shared by build, check, and clippy commands to avoid duplicating the
/// per-crate cache-check + compile + record loop.
///
/// Returns `(recompiled_count, kernel_binary_path, kernel_binary_was_rebuilt)`.
fn compile_kernel_stage(
    prep: &mut BuildPrep,
    mode: CompileMode,
    linker_script: Option<&std::path::Path>,
) -> Result<(usize, Option<PathBuf>, bool)> {
    let total = prep.kernel_crates.len();
    let mut kernel_binary = None;
    let mut kernel_binary_rebuilt = false;
    let mut recompiled = 0;

    let mode_tag = match mode {
        CompileMode::Build => "kernel",
        CompileMode::Check => "check",
        CompileMode::Clippy => "clippy",
    };

    // Quick stage check: if all kernel artifacts exist unchanged and nothing
    // upstream was rebuilt, skip the entire per-crate loop.
    let kernel_names: Vec<String> = prep.kernel_crates.iter().map(|k| k.name.clone()).collect();
    if !prep.force && prep.cache.is_stage_fresh(&kernel_names, &prep.rebuilt) {
        println!("  All {total} crates unchanged, skipping.");
        for krate in &prep.kernel_crates {
            let artifact_path =
                compile::crate_artifact_path(krate, &prep.resolved, None, mode);
            if krate.crate_type == "bin" {
                kernel_binary = Some(artifact_path.clone());
            }
            prep.artifacts.insert(&krate.name, artifact_path);
        }
        return Ok((0, kernel_binary, false));
    }

    for krate in &prep.kernel_crates {
        let artifact_path =
            compile::crate_artifact_path(krate, &prep.resolved, None, mode);
        let dep_info_path = compile::crate_dep_info_path(krate, &prep.resolved, None);
        let dep_names: Vec<String> = krate.deps.iter().map(|d| d.crate_name.clone()).collect();

        let flags_hash = compile::hash_args(&[
            mode_tag.as_ref(),
            krate.name.as_ref(),
            krate.edition.as_ref(),
            krate.crate_type.as_ref(),
            format!("{}", prep.resolved.profile.opt_level).as_ref(),
            prep.target_spec.as_ref(),
        ]);

        if !prep.force {
            if let Some(entry) = prep.cache.entries.get_mut(&krate.name) {
                if entry.is_fresh(&flags_hash, &prep.rebuilt, &dep_names).is_fresh() {
                    println!("  Skipping {} (unchanged)", krate.name);
                    if krate.crate_type == "bin" {
                        kernel_binary = Some(artifact_path.clone());
                    }
                    prep.artifacts.insert(&krate.name, artifact_path);
                    continue;
                }
            }
        }

        let verb = match mode {
            CompileMode::Build => "Compiling",
            CompileMode::Check => "Checking",
            CompileMode::Clippy => "Checking",
        };
        println!("  {verb} {}...", krate.name);
        let artifact = compile::compile_crate(
            krate,
            &prep.resolved,
            &prep.target_spec,
            &prep.sysroot_dir,
            &prep.artifacts,
            Some(&prep.config_rlib),
            None,
            linker_script,
            mode,
        )?;
        if let Ok(entry) = CrateEntry::from_compilation(flags_hash, &artifact, &dep_info_path) {
            prep.cache.entries.insert(krate.name.clone(), entry);
        }
        if krate.crate_type == "bin" {
            kernel_binary = Some(artifact.clone());
            kernel_binary_rebuilt = true;
        }
        prep.rebuilt.insert(krate.name.clone());
        prep.artifacts.insert(&krate.name, artifact);
        recompiled += 1;
    }

    Ok((recompiled, kernel_binary, kernel_binary_rebuilt))
}

/// Shared build logic used by build, run, and test commands.
///
/// Returns the path to the kernel binary (if a bin crate was compiled).
fn do_build(cli: &cli::Cli) -> Result<(config::ResolvedConfig, Option<PathBuf>)> {
    let mut prep = prepare_build(cli)?;

    // Compile kernel crates.
    println!("\nCompiling kernel crates...");
    let linker_script = prep.linker_script.clone();
    let total = prep.kernel_crates.len();
    let (recompiled, kernel_binary, kernel_binary_rebuilt) =
        compile_kernel_stage(&mut prep, CompileMode::Build, linker_script.as_deref())?;

    // Save cache manifest.
    prep.cache.save(&prep.resolved.root)?;

    // Generate HBTF backtrace file (skip if kernel binary wasn't rebuilt).
    if let Some(ref kernel_bin) = kernel_binary {
        if kernel_binary_rebuilt || prep.force {
            let hbtf_path = prep.resolved.root.join("build/backtrace.hbtf");
            println!("\nGenerating HBTF...");
            artifact::hbtf::generate_hbtf(
                kernel_bin,
                &hbtf_path,
                prep.resolved.profile.debug_info,
            )?;

            // Also copy to target/ for cargo-image-runner compatibility.
            let target_hbtf = prep.resolved.root.join("target/backtrace.hbtf");
            std::fs::create_dir_all(target_hbtf.parent().unwrap())?;
            std::fs::copy(&hbtf_path, &target_hbtf)?;
        } else {
            println!("\nHBTF unchanged, skipping.");
        }
    }

    // Build initrd.
    let initrd_path = prep.resolved.root.join("build/initrd.cpio");
    let target_initrd = prep.resolved.root.join("target/initrd.cpio");

    // Resolve userspace crate source roots for initrd freshness tracking.
    let registry = crate_graph::load_crate_registry(&prep.resolved.root)?;
    let sysroot_src = sysroot::sysroot_src_dir()?;
    let user_crates = crate_graph::resolve_and_sort(
        &registry,
        &prep.resolved.root,
        &sysroot_src,
        &CrateContext::Userspace,
    )?;
    let user_source_roots: Vec<PathBuf> = user_crates
        .iter()
        .map(|k| k.root_file.clone())
        .collect();

    // Skip initrd if the output exists, is tracked as fresh in the manifest,
    // and none of the userspace source roots have changed.
    let initrd_fresh = !prep.force
        && prep.cache.is_initrd_fresh(&initrd_path, &user_source_roots)
        && target_initrd.exists();

    if initrd_fresh {
        println!("\nInitrd unchanged, skipping.");
    } else {
        println!("\nBuilding initrd...");
        let built = artifact::initrd::build_initrd(
            &prep.resolved,
            &prep.artifacts,
            &mut prep.cache,
            prep.force,
        )?;

        // Record initrd in the manifest.
        prep.cache.record_initrd(&built, &user_source_roots);
        prep.cache.save(&prep.resolved.root)?;

        std::fs::create_dir_all(target_initrd.parent().unwrap())?;
        std::fs::copy(&built, &target_initrd)?;
    }

    println!("\nBuild complete. ({recompiled} of {total} crates recompiled)");

    Ok((prep.resolved, kernel_binary))
}

/// Build the kernel.
fn cmd_build(cli: &cli::Cli) -> Result<()> {
    let (_, kernel_binary) = do_build(cli)?;

    if let Some(ref kb) = kernel_binary {
        println!("  Kernel: {}", kb.display());
    }
    Ok(())
}

/// Build and run the kernel in QEMU.
fn cmd_run(cli: &cli::Cli, extra_args: &[String]) -> Result<()> {
    let (resolved, kernel_binary) = do_build(cli)?;

    let kernel_bin = kernel_binary.ok_or_else(|| {
        anyhow::anyhow!("no kernel binary produced — check boot-binary in profile")
    })?;

    run::run_kernel(&resolved, &kernel_bin, extra_args)
}

/// Type-check all kernel crates without linking.
fn cmd_check(cli: &cli::Cli) -> Result<()> {
    let mut prep = prepare_build(cli)?;

    println!("\nChecking kernel crates...");
    let total = prep.kernel_crates.len();
    let (recompiled, _, _) = compile_kernel_stage(&mut prep, CompileMode::Check, None)?;

    prep.cache.save(&prep.resolved.root)?;
    println!("\nCheck complete. ({recompiled} of {total} crates checked)");
    Ok(())
}

/// Run clippy lints on project crates.
fn cmd_clippy(cli: &cli::Cli) -> Result<()> {
    let mut prep = prepare_build(cli)?;

    println!("\nLinting kernel crates with clippy...");
    let total = prep.kernel_crates.len();
    let (recompiled, _, _) = compile_kernel_stage(&mut prep, CompileMode::Clippy, None)?;

    prep.cache.save(&prep.resolved.root)?;
    println!("\nClippy complete. ({recompiled} of {total} crates linted)");
    Ok(())
}

/// Run tests.
fn cmd_test(cli: &cli::Cli, args: &cli::TestArgs) -> Result<()> {
    let root = config::find_project_root()?;
    let resolved = config::load_config(
        &root,
        cli.profile.as_deref(),
        cli.target.as_deref(),
    )?;

    let run_host = !args.kernel_only && !args.crash_only;
    let run_kernel = !args.host_only && !args.crash_only;

    // Host tests don't need a full kernel build.
    if run_host {
        test::run_host_tests(&resolved)?;
    }

    // Kernel integration tests require a full build.
    if run_kernel {
        let (resolved, kernel_binary) = do_build(cli)?;

        if let Some(kernel_bin) = kernel_binary {
            run::run_kernel_tests(&resolved, &kernel_bin, &args.extra_args)?;
        } else {
            bail!("no kernel binary produced for integration tests");
        }
    }

    if args.crash_only {
        println!("Crash tests not yet implemented.");
    }

    Ok(())
}

/// Remove build artifacts.
fn cmd_clean() -> Result<()> {
    let root = config::find_project_root()?;
    let build_dir = root.join("build");
    if build_dir.exists() {
        std::fs::remove_dir_all(&build_dir)?;
        println!("Removed {}", build_dir.display());
    } else {
        println!("Nothing to clean.");
    }
    Ok(())
}
