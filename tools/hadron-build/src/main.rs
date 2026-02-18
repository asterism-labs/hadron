//! Hadron kernel build system.
//!
//! A standalone build tool that replaces cargo-xtask and cargo-image-runner.
//! Invokes `rustc` directly, builds a custom sysroot, and provides a
//! Kconfig-like configuration system.

mod analyzer;
mod artifact;
mod cli;
mod compile;
mod config;
mod crate_graph;
mod fmt;
mod run;
mod sysroot;
mod test;

use std::path::PathBuf;

use anyhow::{Result, bail};
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
}

fn prepare_build(cli: &cli::Cli) -> Result<BuildPrep> {
    let root = config::find_project_root()?;
    let resolved = config::load_config(
        &root,
        cli.profile.as_deref(),
        cli.target.as_deref(),
    )?;

    let target_spec_path = root.join(&resolved.target.spec);
    let target_spec = target_spec_path
        .to_str()
        .expect("target spec path is valid UTF-8")
        .to_string();

    // Step 1: Build sysroot.
    println!("Building sysroot for {}...", resolved.target_name);
    let sysroot_output = sysroot::build_sysroot(
        &root,
        &target_spec_path,
        &resolved.target_name,
        resolved.profile.opt_level,
    )?;
    println!("  Sysroot ready.");

    // Step 2: Load crate registry.
    let registry = crate_graph::load_crate_registry(&root)?;
    let sysroot_src = sysroot::sysroot_src_dir()?;

    // Step 3: Compile host crates (proc-macro and dependencies).
    println!("\nCompiling host crates...");
    let mut artifacts = ArtifactMap::default();

    let host_crates =
        crate_graph::resolve_and_sort(&registry, &root, &sysroot_src, &CrateContext::Host)?;

    for krate in &host_crates {
        println!("  Compiling {} (host)...", krate.name);
        let artifact = compile::compile_host_crate(krate, &root, &artifacts)?;
        artifacts.insert(&krate.name, artifact);
    }

    // Step 4: Generate and compile hadron_config.
    println!("\nGenerating hadron_config...");
    let config_rlib = compile::build_config_crate(
        &resolved,
        &target_spec,
        &sysroot_output.sysroot_dir,
    )?;
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
        sysroot_dir: sysroot_output.sysroot_dir,
        artifacts,
        config_rlib,
        kernel_crates,
        linker_script,
    })
}

/// Shared build logic used by build, run, and test commands.
///
/// Returns the path to the kernel binary (if a bin crate was compiled).
fn do_build(cli: &cli::Cli) -> Result<(config::ResolvedConfig, Option<PathBuf>)> {
    let mut prep = prepare_build(cli)?;

    // Compile kernel crates.
    println!("\nCompiling kernel crates...");
    let mut kernel_binary = None;
    for krate in &prep.kernel_crates {
        println!("  Compiling {}...", krate.name);
        let artifact = compile::compile_crate(
            krate,
            &prep.resolved,
            &prep.target_spec,
            &prep.sysroot_dir,
            &prep.artifacts,
            Some(&prep.config_rlib),
            None,
            prep.linker_script.as_deref(),
            CompileMode::Build,
        )?;
        if krate.crate_type == "bin" {
            kernel_binary = Some(artifact.clone());
        }
        prep.artifacts.insert(&krate.name, artifact);
    }

    // Generate HBTF backtrace file.
    if let Some(ref kernel_bin) = kernel_binary {
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
    }

    // Build initrd.
    println!("\nBuilding initrd...");
    let initrd_path = artifact::initrd::build_initrd(&prep.resolved)?;

    // Also copy to target/ for cargo-image-runner compatibility.
    let target_initrd = prep.resolved.root.join("target/initrd.cpio");
    std::fs::create_dir_all(target_initrd.parent().unwrap())?;
    std::fs::copy(&initrd_path, &target_initrd)?;

    Ok((prep.resolved, kernel_binary))
}

/// Build the kernel.
fn cmd_build(cli: &cli::Cli) -> Result<()> {
    let (_, kernel_binary) = do_build(cli)?;

    println!("\nBuild complete.");
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
    for krate in &prep.kernel_crates {
        println!("  Checking {}...", krate.name);
        let artifact = compile::compile_crate(
            krate,
            &prep.resolved,
            &prep.target_spec,
            &prep.sysroot_dir,
            &prep.artifacts,
            Some(&prep.config_rlib),
            None,
            None,
            CompileMode::Check,
        )?;
        prep.artifacts.insert(&krate.name, artifact);
    }

    println!("\nCheck complete.");
    Ok(())
}

/// Run clippy lints on project crates.
fn cmd_clippy(cli: &cli::Cli) -> Result<()> {
    let mut prep = prepare_build(cli)?;

    println!("\nLinting kernel crates with clippy...");
    for krate in &prep.kernel_crates {
        println!("  Checking {}...", krate.name);
        let artifact = compile::compile_crate(
            krate,
            &prep.resolved,
            &prep.target_spec,
            &prep.sysroot_dir,
            &prep.artifacts,
            Some(&prep.config_rlib),
            None,
            None,
            CompileMode::Clippy,
        )?;
        prep.artifacts.insert(&krate.name, artifact);
    }

    println!("\nClippy complete.");
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
