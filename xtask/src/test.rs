//! Test execution for Hadron kernel.
//!
//! Supports two test modes:
//! - **Host unit tests**: Run `cargo test -p <crate>` for each host-testable crate
//!   using the host toolchain (no `--target` or `-Zbuild-std`).
//! - **Integration tests**: Run `cargo test` with build-std flags and the custom
//!   kernel target, executed in QEMU via cargo-image-runner.

use crate::cargo::CargoCommand;
use crate::config::Config;
use anyhow::Result;
use xshell::{Shell, cmd};

/// Crates that compile and pass tests on the host (no kernel target needed).
const HOST_TESTABLE_CRATES: &[&str] = &[
    "hadron-codegen",
    "hadron-dwarf",
    "hadron-elf",
    "noalloc",
    "hadron-core",
    "hadron-driver-api",
    "hadron-drivers",
];

/// Run host-side unit tests for all host-testable crates.
fn run_host_tests(config: &Config) -> Result<()> {
    let sh = Shell::new()?;
    sh.change_dir(&config.workspace_root);

    println!("Running host-side unit tests...");

    for crate_name in HOST_TESTABLE_CRATES {
        println!("  Testing {crate_name}...");
        cmd!(sh, "cargo test -p {crate_name}")
            .run()
            .map_err(|e| anyhow::anyhow!("cargo test -p {crate_name} failed: {e}"))?;
    }

    println!("All host-side unit tests passed.");
    Ok(())
}

/// Run QEMU integration tests using build-std and the custom kernel target.
fn run_integration_tests(
    config: &Config,
    target: &str,
    package: Option<&str>,
    release: bool,
    profile: Option<&str>,
    extra_args: Vec<String>,
) -> Result<()> {
    let package = package.unwrap_or("hadron-kernel");

    println!("Running integration tests for {package} (target: {target})...");

    CargoCommand {
        subcommand: "test".into(),
        target: target.into(),
        package: Some(package.into()),
        release,
        extra_args,
        profile: profile.map(String::from),
    }
    .run(config)
}

/// Entry point: run host tests, integration tests, or both.
pub fn run_tests(
    config: &Config,
    target: &str,
    package: Option<&str>,
    release: bool,
    profile: Option<&str>,
    host_only: bool,
    integration_only: bool,
    extra_args: Vec<String>,
) -> Result<()> {
    let run_host = !integration_only;
    let run_integration = !host_only;

    if run_host {
        run_host_tests(config)?;
    }

    if run_integration {
        run_integration_tests(config, target, package, release, profile, extra_args)?;
    }

    Ok(())
}
