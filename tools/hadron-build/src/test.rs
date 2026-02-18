//! Test execution for the Hadron kernel.
//!
//! Supports:
//! - Host unit tests: `cargo test -p <crate>` for each host-testable crate
//! - Kernel integration tests: build + QEMU via cargo-image-runner (future)

use anyhow::{Context, Result};
use std::process::Command;

use crate::config::ResolvedConfig;

/// Run host-side unit tests for all host-testable crates.
///
/// Uses `cargo test` since these crates compile for the host target.
pub fn run_host_tests(config: &ResolvedConfig) -> Result<()> {
    println!("Running host-side unit tests...");

    for crate_name in &config.tests.host_testable {
        println!("  Testing {crate_name}...");
        let status = Command::new("cargo")
            .arg("test")
            .arg("-p")
            .arg(crate_name)
            .current_dir(&config.root)
            .status()
            .with_context(|| format!("failed to run cargo test -p {crate_name}"))?;

        if !status.success() {
            anyhow::bail!("tests failed for {crate_name}");
        }
    }

    println!("All host-side unit tests passed.");
    Ok(())
}
