//! Kernel build functionality.

use crate::cargo::CargoCommand;
use crate::config::Config;
use anyhow::Result;
use std::path::PathBuf;

/// Build result containing paths to built artifacts.
#[derive(Debug)]
pub struct BuildResult {
    /// Path to the built kernel binary.
    pub kernel_binary: PathBuf,
}

/// Build the kernel, returning the path to the output binary.
pub fn build(
    config: &Config,
    target: &str,
    package: Option<&str>,
    release: bool,
) -> Result<BuildResult> {
    let package = package.unwrap_or("hadron-boot-limine");

    println!("Building {package} for {target}");

    CargoCommand {
        subcommand: "build".into(),
        target: target.into(),
        package: Some(package.into()),
        release,
        extra_args: vec![],
        profile: None,
    }
    .run(config)?;

    let profile = if release { "release" } else { "debug" };
    let kernel_binary = config.target_dir.join(target).join(profile).join(package);

    if !kernel_binary.exists() {
        anyhow::bail!("Built binary not found at: {}", kernel_binary.display());
    }

    Ok(BuildResult { kernel_binary })
}
