//! Hadron build system.
//!
//! A standalone build tool that invokes `rustc` directly, builds a custom
//! sysroot, and provides a Rhai-scripted configuration system.
//!
//! Pipeline: evaluate gluon.rhai → validate model → resolve config →
//!           schedule stages → compile crates → generate artifacts.

mod analyzer;
mod artifact;
mod bench;
mod cache;
mod cli;
mod compile;
mod config;
mod crate_graph;
mod engine;
mod fmt;
mod kconfig;
mod menuconfig;
mod model;
mod model_cache;
mod perf;
mod perf_cmd;
mod run;
mod rustc_cmd;
mod rustc_info;
mod scheduler;
mod sysroot;
mod test;
mod validate;
mod vendor;
mod verbose;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use cache::CacheManifest;
use clap::Parser;
use compile::{ArtifactMap, CompileMode};

fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    verbose::init(cli.verbose);

    match cli.command {
        cli::Command::Configure => cmd_configure(&cli),
        cli::Command::Clean => cmd_clean(),
        cli::Command::Build(ref _args) => cmd_build(&cli),
        cli::Command::Run(ref args) => cmd_run(&cli, &args.extra_args),
        cli::Command::Test(ref args) => cmd_test(&cli, args),
        cli::Command::Bench(ref args) => cmd_bench(&cli, args),
        cli::Command::Check => cmd_check(&cli),
        cli::Command::Clippy => cmd_clippy(&cli),
        cli::Command::Fmt(ref args) => fmt::cmd_fmt(args),
        cli::Command::Menuconfig => cmd_menuconfig(&cli),
        cli::Command::Vendor(ref args) => cmd_vendor(&cli, args),
        cli::Command::Perf(ref args) => perf_cmd::cmd_perf(args),
    }
}

// ===========================================================================
// Model loading
// ===========================================================================

/// Load and validate the build model from `gluon.rhai`.
///
/// If `dependency()` declarations are present, auto-registers vendored crates
/// into the model before validation. Uses the model cache when available.
fn load_model(root: &PathBuf, force: bool) -> Result<model::BuildModel> {
    load_model_inner(root, true, force)
}

/// Load the build model, optionally skipping validation.
///
/// `gluon vendor` uses `validate = false` because vendor directories may not
/// exist yet — the whole point of the command is to create them.
fn load_model_inner(root: &PathBuf, validate: bool, force: bool) -> Result<model::BuildModel> {
    use verbose::vprintln;

    let _t = verbose::Timer::start("model loading (total)");

    // Try the model cache first (unless forced).
    if !force {
        if let Some(model) = model_cache::load_cached_model(root) {
            println!("Using cached build model.");
            return Ok(model);
        }
    }

    println!("Loading gluon.rhai...");

    let mut model = {
        let _t = verbose::Timer::start("script evaluation");
        engine::evaluate_script(root)?
    };

    // Auto-register vendored dependencies if any dependency() declarations exist.
    if !model.dependencies.is_empty() {
        let _t = verbose::Timer::start("vendor dependency resolution");
        let vendor_dir = root.join("vendor");
        let mut version_cache = vendor::VersionCache::new();
        let resolved = vendor::resolve_transitive(&model.dependencies, &vendor_dir, &mut version_cache)?;
        vprintln!("  resolved {} transitive dependencies", resolved.len());

        // Determine the default target from the "default" profile.
        let default_target = model.profiles.get("default")
            .and_then(|p| p.target.clone())
            .unwrap_or_else(|| "x86_64-unknown-hadron".into());

        vendor::auto_register_dependencies(&mut model, &resolved, &vendor_dir, &default_target)?;
    }

    if validate {
        let _t = verbose::Timer::start("model validation");
        validate::validate_model(&model)?;
    }

    // Save the validated model to cache for next time.
    if let Err(e) = model_cache::save_cached_model(root, &model) {
        vprintln!("  warning: failed to save model cache: {e}");
    }

    Ok(model)
}

/// Resolve configuration from the build model.
fn resolve_config(
    cli: &cli::Cli,
) -> Result<(config::ResolvedConfig, model::BuildModel)> {
    use verbose::vprintln;

    let root = config::find_project_root()?;
    let model = load_model(&root, cli.force)?;
    let profile_name = cli.profile.as_deref().unwrap_or("default");
    vprintln!("  resolving config: profile={}, target={}", profile_name,
        cli.target.as_deref().unwrap_or("(from profile)"));
    let resolved = config::resolve_from_model(
        &model,
        profile_name,
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
    let mut state = prepare_pipeline_state(resolved, cli.force, cli.jobs.unwrap_or(0))?;

    println!("\nChecking crates...");
    scheduler::execute_pipeline(&model, &mut state, CompileMode::Check)?;
    state.cache.save(&state.config.root)?;
    println!(
        "\nCheck complete. ({} of {} crates checked)",
        state.recompiled_crates, state.total_crates
    );
    Ok(())
}

/// Run clippy lints on project crates.
fn cmd_clippy(cli: &cli::Cli) -> Result<()> {
    let (resolved, model) = resolve_config(cli)?;
    let mut state = prepare_pipeline_state(resolved, cli.force, cli.jobs.unwrap_or(0))?;

    println!("\nLinting crates with clippy...");
    scheduler::execute_pipeline(&model, &mut state, CompileMode::Clippy)?;
    state.cache.save(&state.config.root)?;
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
        test::run_host_tests(&resolved, cli.jobs.unwrap_or(0))?;
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
    let model = load_model(&root, false)?;
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

/// Vendor external dependencies.
fn cmd_vendor(cli: &cli::Cli, args: &cli::VendorArgs) -> Result<()> {
    let root = config::find_project_root()?;
    // Skip validation: vendor dirs may not exist yet.
    let model = load_model_inner(&root, false, true)?;
    let vendor_dir = root.join("vendor");
    let lock_path = root.join("gluon.lock");

    if model.dependencies.is_empty() {
        println!("No dependency() declarations found in gluon.rhai.");
        return Ok(());
    }

    println!("Resolving {} dependencies...", model.dependencies.len());

    let build_dir = root.join("build");

    if args.check {
        // Verify mode: check lock file and vendor directory match.
        let lock = vendor::read_lock_file(&lock_path)?
            .ok_or_else(|| anyhow::anyhow!("gluon.lock not found — run `gluon vendor` first"))?;

        let mut version_cache = vendor::VersionCache::load(&build_dir);
        let resolved = vendor::resolve_transitive(&model.dependencies, &vendor_dir, &mut version_cache)?;
        let new_lock = vendor::build_lock_file(&resolved, &vendor_dir)?;

        let mut mismatches = 0;
        for new_pkg in &new_lock.packages {
            match lock.packages.iter().find(|p| p.name == new_pkg.name) {
                Some(existing) => {
                    if existing.version != new_pkg.version {
                        println!(
                            "  MISMATCH: {} version {} (lock) vs {} (resolved)",
                            new_pkg.name, existing.version, new_pkg.version
                        );
                        mismatches += 1;
                    }
                    if existing.checksum != new_pkg.checksum {
                        println!(
                            "  MISMATCH: {} checksum differs",
                            new_pkg.name
                        );
                        mismatches += 1;
                    }
                }
                None => {
                    println!("  MISSING: {} not in lock file", new_pkg.name);
                    mismatches += 1;
                }
            }
        }

        // Check for extra packages in lock file.
        for existing in &lock.packages {
            if !new_lock.packages.iter().any(|p| p.name == existing.name) {
                println!("  EXTRA: {} in lock file but not in dependencies", existing.name);
                mismatches += 1;
            }
        }

        if mismatches > 0 {
            anyhow::bail!("{mismatches} mismatch(es) found — run `gluon vendor` to update");
        }

        println!("Lock file and vendor directory are up to date.");
        return Ok(());
    }

    // Fetch missing dependencies. Load persistent cache unless --force.
    let mut version_cache = if cli.force {
        vendor::VersionCache::new()
    } else {
        vendor::VersionCache::load(&build_dir)
    };

    // Prefetch version listings for all crates.io dependencies in parallel.
    let prefetch_batch = cli.jobs.unwrap_or(8).max(1);
    let crates_io_names: Vec<String> = model.dependencies.iter()
        .filter_map(|(name, dep)| match &dep.source {
            model::DepSource::CratesIo { version } if !version.is_empty() => {
                // Only prefetch if version is a requirement, not an exact version.
                if semver::Version::parse(version).is_err() {
                    Some(name.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();
    if !crates_io_names.is_empty() {
        vendor::prefetch_versions(&crates_io_names, &mut version_cache, prefetch_batch)?;
    }

    let mut fetched = 0;
    for (name, dep) in &model.dependencies {
        match &dep.source {
            model::DepSource::CratesIo { version } => {
                if version.is_empty() {
                    anyhow::bail!("dependency '{name}' has no version specified");
                }
                let resolved_version = vendor::resolve_version(name, version, &mut version_cache)?;
                let dest = vendor::find_vendor_dir(name, &vendor_dir);
                if !dest.exists() {
                    vendor::fetch_crates_io(name, &resolved_version, &vendor_dir)?;
                    fetched += 1;
                }
            }
            model::DepSource::Git { url, reference } => {
                let ref_str = match reference {
                    model::GitRef::Rev(r) => r.clone(),
                    model::GitRef::Tag(t) => t.clone(),
                    model::GitRef::Branch(b) => b.clone(),
                    model::GitRef::Default => "HEAD".into(),
                };
                let dest = vendor::find_vendor_dir(name, &vendor_dir);
                if !dest.exists() {
                    vendor::fetch_git(name, url, &ref_str, &vendor_dir)?;
                    fetched += 1;
                }
            }
            model::DepSource::Path { .. } => {
                // Path deps are not vendored.
            }
        }
    }

    // Resolve transitive dependencies and fetch any that are missing.
    // Iterate until all transitive deps are present.
    let mut iterations = 0;
    let max_iterations = 10;
    loop {
        iterations += 1;
        if iterations > max_iterations {
            anyhow::bail!("transitive resolution did not converge after {max_iterations} iterations");
        }

        let resolved = vendor::resolve_transitive(&model.dependencies, &vendor_dir, &mut version_cache)?;

        // Collect deps that need fetching.
        let mut to_fetch: Vec<&vendor::ResolvedDep> = Vec::new();
        for dep in &resolved {
            let vendor_path = vendor::find_vendor_dir(&dep.name, &vendor_dir);
            if !vendor_path.join("Cargo.toml").exists() {
                match &dep.source {
                    vendor::ResolvedSource::CratesIo => {
                        if dep.version.is_empty() {
                            anyhow::bail!(
                                "transitive dependency '{}' has no version — it may need a version in a parent Cargo.toml",
                                dep.name
                            );
                        }
                        to_fetch.push(dep);
                    }
                    vendor::ResolvedSource::Git { .. } => {
                        to_fetch.push(dep);
                    }
                    vendor::ResolvedSource::Path { .. } => {}
                }
            }
        }

        let needed_fetch = !to_fetch.is_empty();
        if needed_fetch {
            // Fetch in parallel. CDN downloads have no meaningful per-IP
            // concurrency limit; default to 12 or the user-specified -j value.
            let batch_size = cli.jobs.unwrap_or(12).max(1);
            for chunk in to_fetch.chunks(batch_size) {
                let results: Vec<Result<PathBuf>> = std::thread::scope(|s| {
                    let handles: Vec<_> = chunk.iter().map(|dep| {
                        let vdir = &vendor_dir;
                        s.spawn(move || -> Result<PathBuf> {
                            match &dep.source {
                                vendor::ResolvedSource::CratesIo => {
                                    vendor::fetch_crates_io(&dep.name, &dep.version, vdir)
                                }
                                vendor::ResolvedSource::Git { url, reference } => {
                                    vendor::fetch_git(&dep.name, url, reference, vdir)
                                }
                                vendor::ResolvedSource::Path { .. } => {
                                    Ok(vdir.join(&dep.name))
                                }
                            }
                        })
                    }).collect();

                    handles.into_iter().map(|h| h.join().unwrap()).collect()
                });

                for result in results {
                    result?;
                }
            }
            fetched += to_fetch.len();
        }

        if !needed_fetch {
            // All deps present — build and write lock file.
            let lock = vendor::build_lock_file(&resolved, &vendor_dir)?;
            vendor::write_lock_file(&lock_path, &lock)?;

            // Persist the version cache for next run.
            if let Err(e) = version_cache.save(&build_dir) {
                verbose::vprintln!("  warning: failed to save version cache: {e}");
            }

            println!("\nVendoring complete:");
            println!("  {} dependencies resolved", resolved.len());
            println!("  {} crates fetched", fetched);
            println!("  Lock file written to {}", lock_path.display());

            if args.prune {
                prune_vendor_dir(&resolved, &vendor_dir)?;
            }

            return Ok(());
        }
    }
}

/// Remove vendored crates not referenced by any resolved dependency.
fn prune_vendor_dir(resolved: &[vendor::ResolvedDep], vendor_dir: &std::path::Path) -> Result<()> {
    let referenced: HashSet<String> = resolved.iter().map(|d| d.name.clone()).collect();
    let mut removed = 0;

    if let Ok(entries) = std::fs::read_dir(vendor_dir) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();

            // Extract the crate name (strip version suffix if present).
            let crate_name = if let Some(idx) = dir_name.rfind('-') {
                let maybe_version = &dir_name[idx + 1..];
                // Heuristic: if the part after the last '-' looks like a version, strip it.
                if maybe_version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    dir_name[..idx].to_string()
                } else {
                    dir_name.clone()
                }
            } else {
                dir_name.clone()
            };

            if !referenced.contains(&crate_name) {
                // Double-check by looking at Cargo.toml name.
                let cargo_toml = entry.path().join("Cargo.toml");
                let actual_name = if cargo_toml.exists() {
                    vendor::parse_cargo_toml(&cargo_toml)
                        .ok()
                        .map(|p| p.package.name)
                } else {
                    None
                };

                let name = actual_name.as_deref().unwrap_or(&crate_name);
                if !referenced.contains(name) {
                    println!("  Pruning unreferenced: {dir_name}");
                    std::fs::remove_dir_all(entry.path())?;
                    removed += 1;
                }
            }
        }
    }

    if removed > 0 {
        println!("  Pruned {removed} unreferenced crate(s).");
    } else {
        println!("  No unreferenced crates to prune.");
    }

    Ok(())
}

/// Run kernel benchmarks.
fn cmd_bench(cli: &cli::Cli, args: &cli::BenchArgs) -> Result<()> {
    let (mut state, model) = do_build(cli)?;
    bench::run_benchmarks(&model, &mut state, args)
}

// ===========================================================================
// Pipeline helpers
// ===========================================================================

/// Initialize pipeline state with empty per-target maps.
///
/// Takes `ResolvedConfig` by value to avoid a redundant field-by-field clone.
fn prepare_pipeline_state(
    resolved: config::ResolvedConfig,
    force: bool,
    max_workers: usize,
) -> Result<scheduler::PipelineState> {
    use verbose::vprintln;

    let rustc_hash = cache::get_rustc_version_hash()?;
    let cache = if force {
        vprintln!("  force build: ignoring cache");
        CacheManifest::new(rustc_hash.clone())
    } else {
        match CacheManifest::load(&resolved.root) {
            Some(m) if m.rustc_version_hash == rustc_hash => {
                vprintln!("  loaded cache manifest ({} entries)", m.entries.len());
                m
            }
            Some(_) => {
                vprintln!("  rustc version changed, discarding cache");
                CacheManifest::new(rustc_hash.clone())
            }
            _ => {
                vprintln!("  no cache manifest found, starting fresh");
                CacheManifest::new(rustc_hash.clone())
            }
        }
    };

    Ok(scheduler::PipelineState {
        config: resolved,
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
        max_workers,
    })
}

/// Shared build logic used by build, run, and test commands.
///
/// Returns the pipeline state (with artifact map, sysroots, etc.) and the
/// build model, so callers like `cmd_test` can compile test binaries.
fn do_build(cli: &cli::Cli) -> Result<(scheduler::PipelineState, model::BuildModel)> {
    let (resolved, model) = resolve_config(cli)?;

    let mut state = prepare_pipeline_state(resolved, cli.force, cli.jobs.unwrap_or(0))?;

    println!("\nCompiling crates...");
    scheduler::execute_pipeline(&model, &mut state, CompileMode::Build)?;

    state.cache.save(&state.config.root)?;
    println!(
        "\nBuild complete. ({} of {} crates recompiled)",
        state.recompiled_crates, state.total_crates
    );

    Ok((state, model))
}
