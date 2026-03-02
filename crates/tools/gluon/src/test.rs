//! Test execution for the Hadron kernel.
//!
//! Supports:
//! - Host unit tests: `cargo test -p <crate>` for each host-testable crate
//! - Kernel integration tests: compile via rustc + run in QEMU via cargo-image-runner

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::artifact::hkif;
use crate::compile::{self, CompileMode};
use crate::config::ResolvedConfig;
use crate::crate_graph;
use crate::model::BuildModel;
use crate::run;
use crate::scheduler::PipelineState;
use crate::sysroot;

/// Result of a single host test crate run.
struct HostTestResult {
    crate_name: String,
    success: bool,
    output: std::process::Output,
}

/// Run host-side unit tests for all host-testable crates.
///
/// Runs crates in parallel using a work-stealing pattern. Each worker captures
/// output to avoid interleaving. All failures are collected and reported at the
/// end.
pub fn run_host_tests(config: &ResolvedConfig, max_workers: usize) -> Result<()> {
    let crates = &config.tests.host_testable;
    if crates.is_empty() {
        println!("No host-testable crates configured.");
        return Ok(());
    }

    let num_workers = match max_workers {
        0 => std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
        n => n,
    };
    // Don't spawn more workers than crates.
    let num_workers = num_workers.min(crates.len());

    println!(
        "Running host-side unit tests ({} crates, {} workers)...",
        crates.len(),
        num_workers
    );

    let root = &config.root;
    let next_idx = std::sync::Mutex::new(0usize);
    let (tx, rx) = std::sync::mpsc::channel::<HostTestResult>();

    std::thread::scope(|s| {
        for _ in 0..num_workers {
            let tx = tx.clone();
            let next = &next_idx;
            s.spawn(move || {
                loop {
                    let idx = {
                        let mut guard = next.lock().unwrap();
                        let i = *guard;
                        if i >= crates.len() {
                            break;
                        }
                        *guard = i + 1;
                        i
                    };

                    let crate_name = &crates[idx];
                    let output = Command::new("cargo")
                        .arg("test")
                        .arg("-p")
                        .arg(crate_name)
                        .current_dir(root)
                        .output();

                    let result = match output {
                        Ok(out) => HostTestResult {
                            crate_name: crate_name.clone(),
                            success: out.status.success(),
                            output: out,
                        },
                        Err(e) => {
                            // Synthesize a failed output.
                            let stderr = format!("failed to run cargo test: {e}");
                            HostTestResult {
                                crate_name: crate_name.clone(),
                                success: false,
                                output: std::process::Output {
                                    status: std::process::ExitStatus::default(),
                                    stdout: Vec::new(),
                                    stderr: stderr.into_bytes(),
                                },
                            }
                        }
                    };

                    if tx.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        // Collect results as they arrive.
        let mut passed = 0usize;
        let mut failures: Vec<HostTestResult> = Vec::new();

        for result in rx {
            if result.success {
                println!("  {} ... ok", result.crate_name);
                passed += 1;
            } else {
                println!("  {} ... FAILED", result.crate_name);
                failures.push(result);
            }
        }

        println!(
            "\nHost test results: {} passed, {} failed",
            passed,
            failures.len()
        );

        if !failures.is_empty() {
            println!("\nFailure details:");
            for f in &failures {
                println!("\n--- {} ---", f.crate_name);
                let stdout = String::from_utf8_lossy(&f.output.stdout);
                let stderr = String::from_utf8_lossy(&f.output.stderr);
                if !stdout.is_empty() {
                    print!("{stdout}");
                }
                if !stderr.is_empty() {
                    eprint!("{stderr}");
                }
            }
            bail!("{} host test crate(s) failed", failures.len());
        }

        Ok(())
    })
}

/// Compile kernel integration test binaries.
///
/// Resolves the test-deps group, compiles dev dependencies that aren't already
/// in the artifact map, discovers test files, and compiles each as a standalone
/// `no_main` binary linked against the crate-under-test and its dependencies.
///
/// Returns a list of `(test_name, binary_path)` pairs.
pub fn compile_kernel_tests(
    model: &BuildModel,
    state: &mut PipelineState,
) -> Result<Vec<(String, PathBuf)>> {
    let crate_name = match &state.config.tests.kernel_tests_crate {
        Some(name) => name.clone(),
        None => bail!("kernel_tests_crate not configured"),
    };
    let tests_dir = match &state.config.tests.kernel_tests_dir {
        Some(dir) => dir.clone(),
        None => bail!("kernel_tests_dir not configured"),
    };
    let linker_script = match &state.config.tests.kernel_tests_linker_script {
        Some(ls) => ls.clone(),
        None => bail!("kernel_tests_linker_script not configured"),
    };

    let krate_def = model
        .crates
        .get(&crate_name)
        .ok_or_else(|| anyhow::anyhow!("kernel_tests_crate '{crate_name}' not found in model"))?;

    // Ensure sysroot and config crate exist for the test target.
    let target = &krate_def.target;
    let target_spec = state
        .target_specs
        .get(target)
        .ok_or_else(|| {
            anyhow::anyhow!("target spec for '{target}' not resolved (run build first)")
        })?
        .clone();
    let sysroot_dir = state
        .sysroots
        .get(target)
        .ok_or_else(|| anyhow::anyhow!("sysroot for '{target}' not built (run build first)"))?
        .clone();

    // Compile test-deps group (hadron-test, etc.) if not already compiled.
    if model.groups.contains_key("test-deps") {
        println!("\nCompiling test dependencies...");
        let sysroot_src = sysroot::sysroot_src_dir()?;
        let resolved_test_deps = crate_graph::resolve_group_from_model(
            model,
            "test-deps",
            &state.config.root,
            &sysroot_src,
        )?;

        let config_rlib = state.config_rlibs.get(target).cloned();

        for krate in &resolved_test_deps {
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
                &[],
            )?;
            state.artifacts.insert(&krate.name, artifact);
        }
    }

    // Discover test files.
    let tests_path = state.config.root.join(&tests_dir);
    let mut test_files: Vec<PathBuf> = Vec::new();
    if tests_path.is_dir() {
        for entry in std::fs::read_dir(&tests_path)
            .with_context(|| format!("reading test dir {}", tests_path.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                test_files.push(path);
            }
        }
    }
    test_files.sort();

    if test_files.is_empty() {
        println!("No kernel test files found in {tests_dir}");
        return Ok(Vec::new());
    }

    println!("\nCompiling {} kernel test binaries...", test_files.len());

    // Build the out directory for test binaries.
    let test_out_dir = state.config.root.join("build/tests");
    std::fs::create_dir_all(&test_out_dir)?;

    // Collect all extern crate flags: regular deps + dev_deps + config crate.
    let config_rlib = state.config_rlibs.get(target).cloned();
    let out_dir = state
        .config
        .root
        .join("build/kernel")
        .join(target)
        .join("debug");

    let mut test_binaries = Vec::new();
    for test_file in &test_files {
        let test_name = test_file.file_stem().unwrap().to_string_lossy().to_string();

        // Pass 1: compile test binary without HKIF.
        println!("  Compiling test {test_name} (pass 1)...");
        let binary = compile_test_binary(
            test_file,
            &test_name,
            krate_def,
            &state.config,
            &target_spec,
            &sysroot_dir,
            &state.artifacts,
            config_rlib.as_deref(),
            &out_dir,
            &test_out_dir,
            &linker_script,
            &[],
        )?;

        // HKIF generation: extract symbols from pass-1 ELF and build HKIF object.
        println!("  Generating HKIF for {test_name}...");
        let hkif_bin = test_out_dir.join(format!("{test_name}.hkif.bin"));
        let hkif_asm = test_out_dir.join(format!("{test_name}.hkif.S"));
        let hkif_obj = test_out_dir.join(format!("{test_name}.hkif.o"));

        hkif::generate_hkif(&binary, &hkif_bin, state.config.profile.debug_info)?;
        hkif::generate_hkif_asm(&hkif_bin, &hkif_asm)?;
        hkif::assemble_hkif(&hkif_asm, &hkif_obj)?;

        // Pass 2: relink test binary with embedded HKIF.
        println!("  Re-linking {test_name} with HKIF...");
        let binary = compile_test_binary(
            test_file,
            &test_name,
            krate_def,
            &state.config,
            &target_spec,
            &sysroot_dir,
            &state.artifacts,
            config_rlib.as_deref(),
            &out_dir,
            &test_out_dir,
            &linker_script,
            &[hkif_obj],
        )?;

        test_binaries.push((test_name, binary));
    }

    println!("All {} kernel test binaries compiled.", test_binaries.len());
    Ok(test_binaries)
}

/// Compile a single kernel integration test binary.
fn compile_test_binary(
    test_file: &Path,
    test_name: &str,
    krate_def: &crate::model::CrateDef,
    config: &ResolvedConfig,
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
    cmd.arg(format!("-Copt-level={}", config.profile.opt_level));
    cmd.arg("-Cforce-frame-pointers=yes");

    if config.profile.debug_info {
        cmd.arg("-Cdebuginfo=2");
    }

    // Target and sysroot.
    cmd.arg("--target").arg(target_spec);
    cmd.arg("--sysroot").arg(sysroot_dir);

    // Search paths for transitive deps and host proc-macros.
    cmd.arg("-L").arg(lib_dir);
    cmd.arg("-L").arg(config.root.join("build/host"));

    // Extern the crate-under-test itself.
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

    // Extern all dev deps.
    for (_extern_name, dep) in &krate_def.dev_deps {
        if let Some(path) = artifacts.get(&dep.crate_name) {
            let extern_name = compile::crate_name_sanitized(&dep.extern_name);
            cmd.arg("--extern")
                .arg(format!("{extern_name}={}", path.display()));
        }
    }

    // Link config crate if available.
    if let Some(config_path) = config_rlib {
        cmd.arg("--extern")
            .arg(format!("hadron_config={}", config_path.display()));
    }

    // Config cfgs for bool options.
    for (name, value) in &config.options {
        if let crate::config::ResolvedValue::Bool(true) = value {
            cmd.arg("--cfg").arg(format!("hadron_{name}"));
        }
    }

    // Linker script and GC sections.
    let ld_path = config.root.join(linker_script);
    cmd.arg(format!("-Clink-arg=-T{}", ld_path.display()));
    cmd.arg("-Clink-arg=--gc-sections");

    // Extra object files (e.g., HKIF blob for pass-2 link).
    for obj in extra_link_objects {
        cmd.arg(format!("-Clink-arg={}", obj.display()));
    }

    // Output.
    cmd.arg("--out-dir").arg(out_dir);

    // Source file.
    cmd.arg(test_file);

    let output = cmd
        .output()
        .with_context(|| format!("failed to run rustc for test {test_name}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to compile test '{test_name}':\n{stderr}");
    }

    let binary = out_dir.join(test_name);
    if !binary.exists() {
        bail!("expected test binary not found: {}", binary.display());
    }

    Ok(binary)
}

/// Run compiled kernel test binaries in QEMU.
///
/// Each test binary is booted individually via `cargo-image-runner`.
/// Reports per-test pass/fail results.
pub fn run_kernel_test_binaries(
    config: &ResolvedConfig,
    binaries: &[(String, PathBuf)],
    extra_args: &[String],
) -> Result<()> {
    if binaries.is_empty() {
        println!("No kernel test binaries to run.");
        return Ok(());
    }

    println!("\nRunning {} kernel integration tests...", binaries.len());

    let mut passed = 0;
    let mut failed = Vec::new();

    for (name, binary) in binaries {
        println!("  Running test {name}...");
        match run::run_kernel_tests(config, binary, extra_args) {
            Ok(()) => {
                println!("  {name}: ok");
                passed += 1;
            }
            Err(e) => {
                println!("  {name}: FAILED ({e:#})");
                failed.push(name.clone());
            }
        }
    }

    println!(
        "\nKernel test results: {} passed, {} failed",
        passed,
        failed.len()
    );

    if !failed.is_empty() {
        println!("Failed tests:");
        for name in &failed {
            println!("  - {name}");
        }
        bail!("{} kernel test(s) failed", failed.len());
    }

    Ok(())
}

// ===========================================================================
// Kernel-internal tests (ktest)
// ===========================================================================

/// Compile the ktest kernel: rebuild key crates with `--cfg ktest`.
///
/// Recompiles hadron-kernel, hadron-drivers, and the boot binary with the
/// `ktest` cfg flag into a separate output directory (`build/kernel/ktest/`),
/// reusing all other pre-compiled artifacts from the normal build.
pub fn compile_ktest_kernel(model: &BuildModel, state: &mut PipelineState) -> Result<PathBuf> {
    let boot_binary_name = &state.config.profile.boot_binary;
    let boot_def = model
        .crates
        .get(boot_binary_name)
        .ok_or_else(|| anyhow::anyhow!("boot binary '{boot_binary_name}' not found in model"))?;

    let target = &boot_def.target;
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
    let config_rlib = state.config_rlibs.get(target).cloned();
    let sysroot_src = sysroot::sysroot_src_dir()?;

    println!("\nCompiling ktest kernel...");

    // Resolve the kernel crate groups to get ResolvedCrate definitions.
    let kernel_libs = crate_graph::resolve_group_from_model(
        model,
        "kernel-libs",
        &state.config.root,
        &sysroot_src,
    )?;
    let kernel_drivers = crate_graph::resolve_group_from_model(
        model,
        "kernel-drivers",
        &state.config.root,
        &sysroot_src,
    )?;
    let kernel_main = crate_graph::resolve_group_from_model(
        model,
        "kernel-main",
        &state.config.root,
        &sysroot_src,
    )?;

    // Build a ktest artifact map starting from the normal build artifacts.
    let mut ktest_artifacts = state.artifacts.clone();

    // Crates that need --cfg ktest: hadron-kernel, hadron-drivers, boot binary.
    let ktest_crate_names = ["hadron-kernel", "hadron-drivers", boot_binary_name.as_str()];
    let ktest_cfgs: &[&str] = &["ktest"];

    // Find and recompile each ktest-affected crate in dependency order.
    let all_resolved: Vec<&crate_graph::ResolvedCrate> = kernel_libs
        .iter()
        .chain(kernel_drivers.iter())
        .chain(kernel_main.iter())
        .collect();

    for crate_name in &ktest_crate_names {
        let resolved = all_resolved
            .iter()
            .find(|c| c.name == *crate_name)
            .ok_or_else(|| anyhow::anyhow!("crate '{crate_name}' not found in resolved groups"))?;

        println!("  Compiling {crate_name} (ktest)...");
        let artifact = compile::compile_crate(
            resolved,
            &state.config,
            Some(&target_spec),
            Some(&sysroot_dir),
            &ktest_artifacts,
            config_rlib.as_deref(),
            Some("ktest"),
            CompileMode::Build,
            ktest_cfgs,
        )?;
        ktest_artifacts.insert(crate_name, artifact);
    }

    // HKIF generation for the ktest boot binary.
    let ktest_out_dir = state.config.root.join("build/kernel/ktest/debug");
    let boot_binary = ktest_out_dir.join(compile::crate_name_sanitized(boot_binary_name));

    println!("  Generating HKIF for ktest kernel...");
    let hkif_bin = ktest_out_dir.join("ktest.hkif.bin");
    let hkif_asm = ktest_out_dir.join("ktest.hkif.S");
    let hkif_obj = ktest_out_dir.join("ktest.hkif.o");

    hkif::generate_hkif(&boot_binary, &hkif_bin, state.config.profile.debug_info)?;
    hkif::generate_hkif_asm(&hkif_bin, &hkif_asm)?;
    hkif::assemble_hkif(&hkif_asm, &hkif_obj)?;

    // Re-link the boot binary with HKIF embedded.
    let boot_resolved = all_resolved
        .iter()
        .find(|c| c.name == *boot_binary_name)
        .unwrap();

    println!("  Re-linking ktest kernel with HKIF...");
    let final_binary = compile::relink_with_objects(
        boot_resolved,
        &state.config,
        &target_spec,
        &sysroot_dir,
        &ktest_artifacts,
        config_rlib.as_deref(),
        &[hkif_obj],
        ktest_cfgs,
        Some("ktest"),
    )?;

    println!("  Ktest kernel: {}", final_binary.display());
    Ok(final_binary)
}

/// Run the ktest kernel in QEMU.
///
/// Boots the ktest kernel binary and checks for test success/failure via
/// the isa-debug-exit device (same as integration tests).
pub fn run_ktest_kernel(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<()> {
    println!("\nRunning ktest kernel...");
    run::run_kernel_tests(config, kernel_binary, extra_args)
}

// ===========================================================================
// Userspace tests
// ===========================================================================

/// Compile userspace test binaries from `userspace/tests/*.rs`.
///
/// Resolves the `utest-deps` group (which provides `hadron-utest`), then
/// compiles each `.rs` file in the configured tests directory as a standalone
/// ELF targeting `x86_64-unknown-hadron-user`.
///
/// Returns `[(test_name, binary_path)]` pairs.
pub fn compile_userspace_tests(
    model: &BuildModel,
    state: &mut PipelineState,
) -> Result<Vec<(String, PathBuf)>> {
    let tests_dir = match &state.config.tests.userspace_tests_dir {
        Some(dir) => dir.clone(),
        None => bail!("userspace_tests_dir not configured"),
    };

    let utest_target = "x86_64-unknown-hadron-user";

    let target_spec = state
        .target_specs
        .get(utest_target)
        .ok_or_else(|| {
            anyhow::anyhow!("target spec for '{utest_target}' not resolved (run build first)")
        })?
        .clone();
    let sysroot_dir = state
        .sysroots
        .get(utest_target)
        .ok_or_else(|| anyhow::anyhow!("sysroot for '{utest_target}' not built (run build first)"))?
        .clone();

    // Compile utest-deps group (hadron-utest, etc.) if not already compiled.
    if model.groups.contains_key("utest-deps") {
        println!("\nCompiling utest dependencies...");
        let sysroot_src = sysroot::sysroot_src_dir()?;
        let resolved_utest_deps = crate_graph::resolve_group_from_model(
            model,
            "utest-deps",
            &state.config.root,
            &sysroot_src,
        )?;

        for krate in &resolved_utest_deps {
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
                None,
                None,
                CompileMode::Build,
                &[],
            )?;
            state.artifacts.insert(&krate.name, artifact);
        }
    }

    // Discover test files.
    let tests_path = state.config.root.join(&tests_dir);
    let mut test_files: Vec<PathBuf> = Vec::new();
    if tests_path.is_dir() {
        for entry in std::fs::read_dir(&tests_path)
            .with_context(|| format!("reading test dir {}", tests_path.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                test_files.push(path);
            }
        }
    }
    test_files.sort();

    if test_files.is_empty() {
        println!("No userspace test files found in {tests_dir}");
        return Ok(Vec::new());
    }

    println!(
        "\nCompiling {} userspace test binaries...",
        test_files.len()
    );

    let out_dir = state.config.root.join("build/utests");
    std::fs::create_dir_all(&out_dir)?;

    // Library search path for userspace rlibs.
    let lib_dir = state
        .config
        .root
        .join("build/kernel")
        .join(utest_target)
        .join("debug");

    let mut test_binaries = Vec::new();
    for test_file in &test_files {
        let test_name = test_file.file_stem().unwrap().to_string_lossy().to_string();
        println!("  Compiling userspace test {test_name}...");

        let binary = compile_utest_binary(
            test_file,
            &test_name,
            &state.config,
            &target_spec,
            &sysroot_dir,
            &lib_dir,
            &out_dir,
        )?;
        test_binaries.push((test_name, binary));
    }

    println!(
        "All {} userspace test binaries compiled.",
        test_binaries.len()
    );
    Ok(test_binaries)
}

/// Compile a single userspace test binary.
fn compile_utest_binary(
    test_file: &Path,
    test_name: &str,
    config: &ResolvedConfig,
    target_spec: &str,
    sysroot_dir: &Path,
    lib_dir: &Path,
    out_dir: &Path,
) -> Result<PathBuf> {
    let mut cmd = Command::new("rustc");
    cmd.arg("--edition=2024");
    cmd.arg("-Zunstable-options"); // required for custom target spec JSON
    cmd.arg("-Cpanic=abort");
    cmd.arg(format!("-Copt-level={}", config.profile.opt_level));

    if config.profile.debug_info {
        cmd.arg("-Cdebuginfo=2");
    }

    // Target and sysroot.
    cmd.arg("--target").arg(target_spec);
    cmd.arg("--sysroot").arg(sysroot_dir);

    // Search path for target-side transitive deps.
    cmd.arg("-L").arg(lib_dir);
    // Proc-macro dylibs (e.g. hadron_syscall_macros) live in the host dir.
    cmd.arg("-L").arg(config.root.join("build/host"));

    // Extern rlibs directly from the lib_dir — more robust than artifact map
    // lookups since gluon crate names and Rust crate names can differ.
    let utest_rlib = lib_dir.join("libhadron_utest.rlib");
    if utest_rlib.exists() {
        cmd.arg("--extern")
            .arg(format!("hadron_utest={}", utest_rlib.display()));
    }

    // The syscall crate is built by gluon as crate name "hadron_syscall_user"
    // (sanitized from "hadron-syscall-user"). hadron_utest's rlib records the
    // dep under that same crate name, so we must use it as the extern alias.
    let syscall_rlib = lib_dir.join("libhadron_syscall_user.rlib");
    if syscall_rlib.exists() {
        cmd.arg("--extern")
            .arg(format!("hadron_syscall_user={}", syscall_rlib.display()));
    }

    // Output directory.
    cmd.arg("--out-dir").arg(out_dir);

    // Source file.
    cmd.arg(test_file);

    let output = cmd
        .output()
        .with_context(|| format!("failed to run rustc for utest {test_name}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to compile utest '{test_name}':\n{stderr}");
    }

    let binary = out_dir.join(test_name);
    if !binary.exists() {
        bail!("expected utest binary not found: {}", binary.display());
    }

    Ok(binary)
}

/// Run compiled userspace test binaries in QEMU.
///
/// For each binary, builds a minimal CPIO (binary as `/bin/init`) and boots
/// the standard kernel with `--utest` on the cmdline. QEMU exits 33 on pass
/// (PID 1 exits 0) or 35 on fail.
pub fn run_userspace_test_binaries(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    binaries: &[(String, PathBuf)],
    extra_args: &[String],
) -> Result<()> {
    if binaries.is_empty() {
        println!("No userspace test binaries to run.");
        return Ok(());
    }

    println!("\nRunning {} userspace tests...", binaries.len());

    let mut passed = 0;
    let mut failed = Vec::new();

    for (name, binary) in binaries {
        println!("  Running userspace test {name}...");
        let cpio = crate::artifact::utest_cpio::build_utest_cpio(&config.root, name, binary)?;
        match run::run_userspace_test(config, kernel_binary, name, &cpio, extra_args) {
            Ok(()) => {
                println!("  {name}: ok");
                passed += 1;
            }
            Err(e) => {
                println!("  {name}: FAILED ({e:#})");
                failed.push(name.clone());
            }
        }
    }

    println!(
        "\nUserspace test results: {} passed, {} failed",
        passed,
        failed.len()
    );

    if !failed.is_empty() {
        println!("Failed tests:");
        for name in &failed {
            println!("  - {name}");
        }
        bail!("{} userspace test(s) failed", failed.len());
    }

    Ok(())
}
