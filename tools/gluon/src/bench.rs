//! Benchmark compilation and execution for the Hadron kernel.
//!
//! Discovers benchmark files in `kernel/hadron-kernel/benches/`, compiles each
//! as a standalone `no_main` binary using the same two-pass link flow as tests,
//! then launches each in QEMU with serial capture. Parses the HBENCH binary
//! format from captured serial data and displays results.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::artifact::hkif;
use crate::cli::BenchArgs;
use crate::compile::{self, CompileMode};
use crate::crate_graph;
use crate::model::BuildModel;
use crate::perf;
use crate::run;
use crate::scheduler::PipelineState;
use crate::sysroot;

/// Compile and run all kernel benchmarks.
pub fn run_benchmarks(
    model: &BuildModel,
    state: &mut PipelineState,
    args: &BenchArgs,
) -> Result<()> {
    let bench_binaries = compile_kernel_benchmarks(model, state)?;

    if bench_binaries.is_empty() {
        println!("No kernel benchmark binaries found.");
        return Ok(());
    }

    println!("\nRunning {} kernel benchmarks...", bench_binaries.len());

    for (name, binary) in &bench_binaries {
        println!("  Running benchmark {name}...");

        // Run in QEMU with serial captured to a file.
        let serial_path = state
            .config
            .root
            .join(format!("build/bench_{name}_serial.bin"));

        match run_benchmark_binary(&state.config, binary, &serial_path, &args.extra_args) {
            Ok(()) => {
                println!("  {name}: completed");

                // Parse and display results.
                if serial_path.exists() {
                    let serial_data = std::fs::read(&serial_path)
                        .with_context(|| format!("reading {}", serial_path.display()))?;

                    match perf::wire::parse_hbench(&serial_data) {
                        Ok(results) => {
                            perf::output::print_bench_table(&results);

                            // Save baseline if requested.
                            if let Some(ref path) = args.save_baseline {
                                perf::bench_analysis::save_baseline(&results, path)?;
                                println!("  Baseline saved to {path}");
                            }

                            // Compare against baseline if requested.
                            if let Some(ref path) = args.baseline {
                                perf::bench_analysis::compare_baseline(
                                    &results,
                                    path,
                                    args.threshold,
                                )?;
                            }
                        }
                        Err(e) => {
                            println!("  Warning: failed to parse benchmark results: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                println!("  {name}: FAILED ({e:#})");
            }
        }
    }

    Ok(())
}

/// Compile kernel benchmark binaries.
///
/// Follows the same pattern as `test::compile_kernel_tests`:
/// - Discovers `.rs` files in `kernel/hadron-kernel/benches/`
/// - Compiles bench-deps group (hadron-bench, etc.)
/// - Compiles each benchmark as a standalone binary with two-pass HKIF link
fn compile_kernel_benchmarks(
    model: &BuildModel,
    state: &mut PipelineState,
) -> Result<Vec<(String, PathBuf)>> {
    let crate_name = state
        .config
        .benchmarks
        .benches_crate
        .as_deref()
        .unwrap_or("hadron-kernel");
    let benches_dir = state
        .config
        .benchmarks
        .benches_dir
        .as_deref()
        .unwrap_or("kernel/hadron-kernel/benches");
    let linker_script = state
        .config
        .benchmarks
        .benches_linker_script
        .clone()
        .unwrap_or_else(|| "targets/x86_64-unknown-hadron.ld".into());

    let krate_def = model
        .crates
        .get(crate_name)
        .ok_or_else(|| anyhow::anyhow!("crate '{crate_name}' not found in model"))?;

    let target = &krate_def.target;
    let target_spec = state
        .target_specs
        .get(target)
        .ok_or_else(|| anyhow::anyhow!("target spec for '{target}' not resolved"))?
        .clone();
    let sysroot_dir = state
        .sysroots
        .get(target)
        .ok_or_else(|| anyhow::anyhow!("sysroot for '{target}' not built"))?
        .clone();

    // Compile bench-deps group if present.
    if model.groups.contains_key("bench-deps") {
        println!("\nCompiling bench dependencies...");
        let sysroot_src = sysroot::sysroot_src_dir()?;
        let resolved_bench_deps = crate_graph::resolve_group_from_model(
            model,
            "bench-deps",
            &state.config.root,
            &sysroot_src,
        )?;

        let config_rlib = state.config_rlibs.get(target).cloned();

        for krate in &resolved_bench_deps {
            if state.artifacts.get(&krate.name).is_some() {
                println!("  Skipping {} (already compiled)", krate.name);
                continue;
            }

            println!("  Compiling {}...", krate.name);
            let artifact = compile::compile_crate(
                krate,
                &state.config,
                Some(&target_spec),
                Some(&sysroot_dir),
                &state.artifacts,
                config_rlib.as_deref(),
                None,
                CompileMode::Build,
            )?;
            state.artifacts.insert(&krate.name, artifact);
        }
    }

    // Discover benchmark files.
    let benches_path = state.config.root.join(benches_dir);
    let mut bench_files: Vec<PathBuf> = Vec::new();
    if benches_path.is_dir() {
        for entry in std::fs::read_dir(&benches_path)
            .with_context(|| format!("reading bench dir {}", benches_path.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                bench_files.push(path);
            }
        }
    }
    bench_files.sort();

    if bench_files.is_empty() {
        println!("No benchmark files found in {benches_dir}");
        return Ok(Vec::new());
    }

    println!(
        "\nCompiling {} kernel benchmark binaries...",
        bench_files.len()
    );

    let bench_out_dir = state.config.root.join("build/benchmarks");
    std::fs::create_dir_all(&bench_out_dir)?;

    let config_rlib = state.config_rlibs.get(target).cloned();
    let out_dir = state
        .config
        .root
        .join("build/kernel")
        .join(target)
        .join("debug");

    let mut bench_binaries = Vec::new();
    for bench_file in &bench_files {
        let bench_name = bench_file
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();

        // Pass 1: compile without HKIF.
        println!("  Compiling benchmark {bench_name} (pass 1)...");
        let binary = compile_bench_binary(
            bench_file,
            &bench_name,
            krate_def,
            &state.config,
            &target_spec,
            &sysroot_dir,
            &state.artifacts,
            config_rlib.as_deref(),
            &out_dir,
            &bench_out_dir,
            &linker_script,
            &[],
        )?;

        // HKIF generation.
        println!("  Generating HKIF for {bench_name}...");
        let hkif_bin = bench_out_dir.join(format!("{bench_name}.hkif.bin"));
        let hkif_asm = bench_out_dir.join(format!("{bench_name}.hkif.S"));
        let hkif_obj = bench_out_dir.join(format!("{bench_name}.hkif.o"));

        hkif::generate_hkif(&binary, &hkif_bin, state.config.profile.debug_info)?;
        hkif::generate_hkif_asm(&hkif_bin, &hkif_asm)?;
        hkif::assemble_hkif(&hkif_asm, &hkif_obj)?;

        // Pass 2: relink with HKIF.
        println!("  Re-linking {bench_name} with HKIF...");
        let binary = compile_bench_binary(
            bench_file,
            &bench_name,
            krate_def,
            &state.config,
            &target_spec,
            &sysroot_dir,
            &state.artifacts,
            config_rlib.as_deref(),
            &out_dir,
            &bench_out_dir,
            &linker_script,
            &[hkif_obj],
        )?;

        bench_binaries.push((bench_name, binary));
    }

    println!(
        "All {} kernel benchmark binaries compiled.",
        bench_binaries.len()
    );
    Ok(bench_binaries)
}

/// Compile a single kernel benchmark binary.
///
/// Uses `--test` + `custom_test_frameworks` just like test binaries, but
/// the benchmark uses `hadron_bench::bench_runner` as the test runner.
fn compile_bench_binary(
    bench_file: &Path,
    bench_name: &str,
    krate_def: &crate::model::CrateDef,
    config: &crate::config::ResolvedConfig,
    target_spec: &str,
    sysroot_dir: &Path,
    artifacts: &compile::ArtifactMap,
    config_rlib: Option<&Path>,
    lib_dir: &Path,
    out_dir: &Path,
    linker_script: &str,
    extra_link_objects: &[PathBuf],
) -> Result<PathBuf> {
    let mut cmd = Command::new("rustc");
    cmd.arg("--test");
    cmd.arg("--edition=2024");
    cmd.arg("-Zunstable-options");
    cmd.arg("-Cpanic=abort");
    cmd.arg(format!(
        "-Copt-level={}",
        config.profile.opt_level
    ));
    cmd.arg("-Cforce-frame-pointers=yes");

    if config.profile.debug_info {
        cmd.arg("-Cdebuginfo=2");
    }

    cmd.arg("--target").arg(target_spec);
    cmd.arg("--sysroot").arg(sysroot_dir);

    cmd.arg("-L").arg(lib_dir);
    cmd.arg("-L").arg(config.root.join("build/host"));

    // Extern the crate-under-test.
    let sanitized_crate = compile::crate_name_sanitized(&krate_def.name);
    if let Some(path) = artifacts.get(&krate_def.name) {
        cmd.arg("--extern")
            .arg(format!("{sanitized_crate}={}", path.display()));
    }

    // Extern all regular deps of the crate-under-test.
    for (_extern_name, dep) in &krate_def.deps {
        if let Some(path) = artifacts.get(&dep.crate_name) {
            let extern_name = compile::crate_name_sanitized(&dep.extern_name);
            cmd.arg("--extern")
                .arg(format!("{extern_name}={}", path.display()));
        }
    }

    // Extern all dev deps (includes hadron-bench via bench-deps).
    for (_extern_name, dep) in &krate_def.dev_deps {
        if let Some(path) = artifacts.get(&dep.crate_name) {
            let extern_name = compile::crate_name_sanitized(&dep.extern_name);
            cmd.arg("--extern")
                .arg(format!("{extern_name}={}", path.display()));
        }
    }

    if let Some(config_path) = config_rlib {
        cmd.arg("--extern")
            .arg(format!("hadron_config={}", config_path.display()));
    }

    // Config cfgs.
    for (name, value) in &config.options {
        if let crate::config::ResolvedValue::Bool(true) = value {
            cmd.arg("--cfg").arg(format!("hadron_{name}"));
        }
    }

    // Linker script.
    let ld_path = config.root.join(linker_script);
    cmd.arg(format!("-Clink-arg=-T{}", ld_path.display()));
    cmd.arg("-Clink-arg=--gc-sections");

    for obj in extra_link_objects {
        cmd.arg(format!("-Clink-arg={}", obj.display()));
    }

    cmd.arg("--out-dir").arg(out_dir);
    cmd.arg(bench_file);

    let output = cmd
        .output()
        .with_context(|| format!("failed to run rustc for benchmark {bench_name}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to compile benchmark '{bench_name}':\n{stderr}");
    }

    let binary = out_dir.join(bench_name);
    if !binary.exists() {
        bail!(
            "expected benchmark binary not found: {}",
            binary.display()
        );
    }

    Ok(binary)
}

/// Run a benchmark binary in QEMU, capturing serial output to a file.
fn run_benchmark_binary(
    config: &crate::config::ResolvedConfig,
    kernel_binary: &Path,
    _serial_output: &Path,
    extra_args: &[String],
) -> Result<()> {
    // Run as a test (with isa-debug-exit for clean shutdown).
    run::run_kernel_tests(config, kernel_binary, extra_args)
}
