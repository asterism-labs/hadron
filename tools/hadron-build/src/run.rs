//! QEMU invocation for running and testing the kernel.
//!
//! Uses `cargo-image-runner` as a library for ISO creation and QEMU execution.
//! This avoids the exit-code wrapping problem that occurs when invoking the CLI:
//! QEMU exits with code 33 (success via `isa-debug-exit`), but the CLI wrapper
//! treats non-zero as failure. Using the library API gives direct access to the
//! runner result.

use anyhow::{Context, Result, bail};
use std::path::Path;

use cargo_image_runner::{BootType, BootloaderKind, ImageFormat};

use crate::config::ResolvedConfig;

/// Strip QEMU flags that the runner already manages internally.
///
/// `QemuRunner` hardcodes `-serial mon:stdio`, so any `-serial <arg>` in the
/// user-provided extra args would cause a "cannot use stdio by multiple
/// character devices" conflict.
fn strip_runner_managed_args(args: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "-serial" {
            skip_next = true;
            continue;
        }
        result.push(arg.clone());
    }
    result
}

/// Build a [`cargo_image_runner::Config`] from hadron's [`ResolvedConfig`].
///
/// Maps hadron config fields to `cargo-image-runner` config fields, applying
/// profile overrides for memory, cores, extra args, and test timeout.
fn build_runner_config(config: &ResolvedConfig, is_test: bool) -> cargo_image_runner::Config {
    let memory = config.profile.qemu_memory.unwrap_or(config.qemu.memory);
    let cores = config.profile.qemu_cores.unwrap_or(1);

    let mut qemu_extra_args = config.qemu.extra_args.clone();
    if let Some(ref profile_args) = config.profile.qemu_extra_args {
        qemu_extra_args.extend(profile_args.iter().cloned());
    }
    // The runner adds -serial mon:stdio internally; strip any -serial from
    // the user-provided args to avoid duplicate stdio device errors.
    let qemu_extra_args = strip_runner_managed_args(&qemu_extra_args);

    let mut cfg = cargo_image_runner::Config::default();
    cfg.boot.boot_type = BootType::Bios;
    cfg.bootloader.kind = BootloaderKind::Limine;
    cfg.bootloader.limine.version = "v10.7.0-binary".into();
    cfg.bootloader.config_file = config.bootloader.config_file.as_ref().map(Into::into);
    cfg.image.format = ImageFormat::Iso;
    cfg.runner.qemu.machine = config.qemu.machine.clone();
    cfg.runner.qemu.memory = memory;
    cfg.runner.qemu.cores = cores;
    cfg.runner.qemu.kvm = false;
    cfg.runner.qemu.extra_args = qemu_extra_args;

    if is_test {
        let test_cfg = &config.qemu.test;
        cfg.test.success_exit_code = Some(test_cfg.success_exit_code as i32);
        let timeout = config.profile.test_timeout.unwrap_or(test_cfg.timeout);
        cfg.test.timeout = Some(u64::from(timeout));
        cfg.test.extra_args = test_cfg.extra_args.clone();
    }

    cfg.extra_files = config
        .image
        .extra_files
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    cfg
}

/// Run the kernel in QEMU.
///
/// Builds an ISO image via `cargo-image-runner` (Limine bootloader, BIOS boot)
/// and launches QEMU. Extra args are forwarded directly to QEMU.
pub fn run_kernel(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<()> {
    let cfg = build_runner_config(config, false);

    println!("Running kernel via cargo-image-runner...");
    cargo_image_runner::builder()
        .with_config(cfg)
        .workspace_root(config.root.clone())
        .executable(kernel_binary.to_path_buf())
        .extra_args(extra_args.to_vec())
        .limine()
        .iso_image()
        .qemu()
        .run()
        .context("failed to run kernel in QEMU")
}

/// Run a kernel integration test in QEMU.
///
/// Builds an ISO image, then runs QEMU with test configuration (isa-debug-exit
/// device, timeout, headless display). Uses the lower-level runner API to
/// override `is_test` detection — our test binaries lack the hex hash suffix
/// that auto-detection expects.
///
/// Returns `Ok(())` if the test exits with the configured success exit code.
pub fn run_kernel_tests(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<()> {
    let mut cfg = build_runner_config(config, true);

    // Extra args from the caller go into runner.qemu.extra_args since
    // cli_extra_args aren't forwarded in test mode.
    cfg.runner
        .qemu
        .extra_args
        .extend(extra_args.iter().cloned());

    // Step 1: Build the ISO image via the builder pipeline.
    let runner = cargo_image_runner::builder()
        .with_config(cfg.clone())
        .workspace_root(config.root.clone())
        .executable(kernel_binary.to_path_buf())
        .limine()
        .iso_image()
        .qemu()
        .build()
        .context("failed to build image runner")?;
    let image_path = runner.build_image().context("failed to build ISO image")?;

    // Step 2: Create context with is_test = true to enable test-specific
    // behavior (timeout enforcement, exit code checking).
    let mut ctx = cargo_image_runner::core::Context::new(
        cfg,
        config.root.clone(),
        kernel_binary.to_path_buf(),
    )
    .context("failed to create runner context")?;
    ctx.is_test = true;

    // Step 3: Run QEMU and get the result.
    use cargo_image_runner::runner::Runner;
    let result = cargo_image_runner::runner::qemu::QemuRunner::new()
        .run(&ctx, &image_path)
        .context("failed to run QEMU")?;

    // Step 4: Check result.
    if result.timed_out {
        bail!("kernel test timed out");
    }
    if !result.success {
        bail!("kernel test failed (exit code {})", result.exit_code);
    }
    Ok(())
}
