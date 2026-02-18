//! QEMU invocation for running and testing the kernel.
//!
//! Delegates to `cargo-image-runner` for ISO creation and QEMU execution.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

use crate::config::ResolvedConfig;

/// Run the kernel in QEMU.
///
/// Requires `cargo-image-runner` to be installed. The runner reads
/// its configuration from `Cargo.toml` workspace metadata.
pub fn run_kernel(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<()> {
    // cargo-image-runner takes the binary path as first arg.
    let mut cmd = Command::new("cargo-image-runner");
    cmd.arg(kernel_binary);
    cmd.current_dir(&config.root);

    // Set profile if the resolved config uses a non-default profile.
    if config.profile.name != "default" {
        cmd.env("CARGO_IMAGE_RUNNER_PROFILE", &config.profile.name);
    }

    for arg in extra_args {
        cmd.arg(arg);
    }

    println!("Running kernel via cargo-image-runner...");
    let status = cmd
        .status()
        .context("failed to run cargo-image-runner — is it installed?")?;

    if !status.success() {
        bail!("cargo-image-runner exited with {status}");
    }
    Ok(())
}

/// Run kernel integration tests in QEMU.
///
/// Invokes cargo-image-runner with test configuration (isa-debug-exit device,
/// timeout, headless display).
pub fn run_kernel_tests(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<()> {
    let test_cfg = &config.qemu.test;

    let mut cmd = Command::new("cargo-image-runner");
    cmd.arg(kernel_binary);
    cmd.current_dir(&config.root);

    // Set test profile.
    cmd.env("CARGO_IMAGE_RUNNER_PROFILE", "");
    // Pass test-specific QEMU args.
    for arg in &test_cfg.extra_args {
        cmd.arg(arg);
    }
    for arg in extra_args {
        cmd.arg(arg);
    }

    println!("Running kernel tests via cargo-image-runner...");
    let status = cmd
        .status()
        .context("failed to run cargo-image-runner — is it installed?")?;

    // cargo-image-runner maps the exit code.
    if !status.success() {
        let code = status.code().unwrap_or(-1);
        if code == test_cfg.success_exit_code as i32 {
            println!("Kernel tests passed (exit code {code}).");
        } else {
            bail!("kernel tests failed (exit code {code})");
        }
    }
    Ok(())
}
