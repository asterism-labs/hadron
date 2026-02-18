//! Hadron build system.
//!
//! A standalone build tool that invokes `rustc` directly, builds a custom
//! sysroot, and provides a Rhai-scripted configuration system.
//!
//! Pipeline: evaluate gluon.rhai → validate model → resolve config →
//!           schedule stages → compile crates → generate artifacts.

mod analyzer;
mod artifact;
mod cache;
mod cli;
mod compile;
mod config;
mod crate_graph;
mod engine;
mod fmt;
mod menuconfig;
mod model;
mod run;
mod scheduler;
mod sysroot;
mod test;
mod validate;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use cache::CacheManifest;
use clap::Parser;
use compile::{ArtifactMap, CompileMode};

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
        cli::Command::Menuconfig => cmd_menuconfig(&cli),
        cli::Command::Vendor => {
            todo!("vendor command")
        }
    }
}

// ===========================================================================
// Model loading
// ===========================================================================

/// Load and validate the build model from `gluon.rhai`.
fn load_model(root: &PathBuf) -> Result<model::BuildModel> {
    println!("Loading gluon.rhai...");
    let model = engine::evaluate_script(root)?;
    validate::validate_model(&model)?;
    Ok(model)
}

/// Resolve configuration from the build model.
fn resolve_config(
    cli: &cli::Cli,
) -> Result<(config::ResolvedConfig, model::BuildModel)> {
    let root = config::find_project_root()?;
    let model = load_model(&root)?;
    let resolved = config::resolve_from_model(
        &model,
        cli.profile.as_deref().unwrap_or("default"),
        cli.target.as_deref(),
        &root,
    )?;
    Ok((resolved, model))
}

// ===========================================================================
// Commands
// ===========================================================================

/// Resolve configuration, print it, and generate rust-project.json.
fn cmd_configure(cli: &cli::Cli) -> Result<()> {
    let (resolved, model) = resolve_config(cli)?;

    config::print_resolved(&resolved);

    // Generate rust-project.json for rust-analyzer.
    println!("\nGenerating rust-project.json...");
    analyzer::generate_rust_project(&resolved, &model)?;

    println!("\nConfiguration resolved successfully.");
    Ok(())
}

/// Build the kernel.
fn cmd_build(cli: &cli::Cli) -> Result<()> {
    let (state, _model) = do_build(cli)?;

    if let Some(ref kb) = state.kernel_binary {
        println!("  Kernel: {}", kb.display());
    }
    Ok(())
}

/// Build and run the kernel in QEMU.
fn cmd_run(cli: &cli::Cli, extra_args: &[String]) -> Result<()> {
    let (state, _model) = do_build(cli)?;

    let kernel_bin = state.kernel_binary.as_ref().ok_or_else(|| {
        anyhow::anyhow!("no kernel binary produced — check boot-binary in profile")
    })?;

    run::run_kernel(&state.config, kernel_bin, extra_args)
}

/// Type-check all kernel crates without linking.
fn cmd_check(cli: &cli::Cli) -> Result<()> {
    let (resolved, model) = resolve_config(cli)?;
    let mut state = prepare_pipeline_state(&resolved, cli.force)?;

    println!("\nChecking crates...");
    scheduler::execute_pipeline(&model, &mut state, CompileMode::Check)?;
    state.cache.save(&resolved.root)?;
    println!(
        "\nCheck complete. ({} of {} crates checked)",
        state.recompiled_crates, state.total_crates
    );
    Ok(())
}

/// Run clippy lints on project crates.
fn cmd_clippy(cli: &cli::Cli) -> Result<()> {
    let (resolved, model) = resolve_config(cli)?;
    let mut state = prepare_pipeline_state(&resolved, cli.force)?;

    println!("\nLinting crates with clippy...");
    scheduler::execute_pipeline(&model, &mut state, CompileMode::Clippy)?;
    state.cache.save(&resolved.root)?;
    println!(
        "\nClippy complete. ({} of {} crates linted)",
        state.recompiled_crates, state.total_crates
    );
    Ok(())
}

/// Run tests.
fn cmd_test(cli: &cli::Cli, args: &cli::TestArgs) -> Result<()> {
    let run_host = !args.kernel_only && !args.crash_only;
    let run_kernel = !args.host_only && !args.crash_only;

    if run_host {
        let (resolved, _model) = resolve_config(cli)?;
        test::run_host_tests(&resolved)?;
    }

    if run_kernel {
        let (mut state, model) = do_build(cli)?;
        let test_binaries = test::compile_kernel_tests(&model, &mut state)?;
        test::run_kernel_test_binaries(&state.config, &test_binaries, &args.extra_args)?;
    }

    if args.crash_only {
        println!("Crash tests not yet implemented.");
    }

    Ok(())
}

/// Interactive TUI menuconfig.
fn cmd_menuconfig(cli: &cli::Cli) -> Result<()> {
    let root = config::find_project_root()?;
    let model = load_model(&root)?;
    menuconfig::run_menuconfig(&model, &root, cli.profile.as_deref().unwrap_or("default"))
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

// ===========================================================================
// Pipeline helpers
// ===========================================================================

/// Initialize pipeline state with empty per-target maps.
fn prepare_pipeline_state(
    resolved: &config::ResolvedConfig,
    force: bool,
) -> Result<scheduler::PipelineState> {
    let rustc_hash = cache::get_rustc_version_hash()?;
    let cache = if force {
        CacheManifest::new(rustc_hash.clone())
    } else {
        match CacheManifest::load(&resolved.root) {
            Some(m) if m.rustc_version_hash == rustc_hash => m,
            _ => CacheManifest::new(rustc_hash.clone()),
        }
    };

    Ok(scheduler::PipelineState {
        config: config::ResolvedConfig {
            project: config::ProjectMeta {
                name: resolved.project.name.clone(),
                version: resolved.project.version.clone(),
            },
            root: resolved.root.clone(),
            target_name: resolved.target_name.clone(),
            target: config::TargetConfig {
                spec: resolved.target.spec.clone(),
            },
            options: resolved.options.clone(),
            profile: config::ResolvedProfile {
                name: resolved.profile.name.clone(),
                target: resolved.profile.target.clone(),
                opt_level: resolved.profile.opt_level,
                debug_info: resolved.profile.debug_info,
                lto: resolved.profile.lto.clone(),
                boot_binary: resolved.profile.boot_binary.clone(),
                qemu_memory: resolved.profile.qemu_memory,
                qemu_cores: resolved.profile.qemu_cores,
                qemu_extra_args: resolved.profile.qemu_extra_args.clone(),
                test_timeout: resolved.profile.test_timeout,
            },
            qemu: config::QemuConfig {
                machine: resolved.qemu.machine.clone(),
                memory: resolved.qemu.memory,
                extra_args: resolved.qemu.extra_args.clone(),
                test: config::QemuTestConfig {
                    success_exit_code: resolved.qemu.test.success_exit_code,
                    timeout: resolved.qemu.test.timeout,
                    extra_args: resolved.qemu.test.extra_args.clone(),
                },
            },
            bootloader: config::BootloaderConfig {
                kind: resolved.bootloader.kind.clone(),
                config_file: resolved.bootloader.config_file.clone(),
            },
            image: config::ImageConfig {
                extra_files: resolved.image.extra_files.clone(),
            },
            tests: config::TestsConfig {
                host_testable: resolved.tests.host_testable.clone(),
                kernel_tests_dir: resolved.tests.kernel_tests_dir.clone(),
                kernel_tests_crate: resolved.tests.kernel_tests_crate.clone(),
                kernel_tests_linker_script: resolved.tests.kernel_tests_linker_script.clone(),
                crash: resolved.tests.crash.iter().map(|ct| config::CrashTest {
                    name: ct.name.clone(),
                    source: ct.source.clone(),
                    expected_exit: ct.expected_exit,
                    expect_output: ct.expect_output.clone(),
                }).collect(),
            },
        },
        target_specs: HashMap::new(),
        sysroots: HashMap::new(),
        config_rlibs: HashMap::new(),
        artifacts: ArtifactMap::default(),
        cache,
        rebuilt: HashSet::new(),
        force,
        kernel_binary: None,
        kernel_binary_rebuilt: false,
        total_crates: 0,
        recompiled_crates: 0,
    })
}

/// Shared build logic used by build, run, and test commands.
///
/// Returns the pipeline state (with artifact map, sysroots, etc.) and the
/// build model, so callers like `cmd_test` can compile test binaries.
fn do_build(cli: &cli::Cli) -> Result<(scheduler::PipelineState, model::BuildModel)> {
    let (resolved, model) = resolve_config(cli)?;

    let mut state = prepare_pipeline_state(&resolved, cli.force)?;

    println!("\nCompiling crates...");
    scheduler::execute_pipeline(&model, &mut state, CompileMode::Build)?;

    state.cache.save(&state.config.root)?;
    println!(
        "\nBuild complete. ({} of {} crates recompiled)",
        state.recompiled_crates, state.total_crates
    );

    Ok((state, model))
}
