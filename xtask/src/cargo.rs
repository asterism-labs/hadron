//! Common cargo invocation with build-std flags.

use crate::config::Config;
use anyhow::{Context, Result};
use xshell::{Shell, cmd};

/// Arguments common to all kernel-target cargo commands.
pub struct CargoCommand {
    /// Cargo subcommand: "build", "run", "test", "check", "clippy", "doc".
    pub subcommand: String,
    /// Target triple, e.g. "x86_64-unknown-hadron".
    pub target: String,
    /// Package to operate on (-p flag). If `None`, no -p is passed.
    pub package: Option<String>,
    /// Whether to pass --release.
    pub release: bool,
    /// Extra arguments appended after `--`.
    pub extra_args: Vec<String>,
    /// Image-runner profile preset (sets `CARGO_IMAGE_RUNNER_PROFILE`).
    pub profile: Option<String>,
}

impl CargoCommand {
    /// Execute the cargo command with build-std flags.
    pub fn run(&self, config: &Config) -> Result<()> {
        let sh = Shell::new()?;
        sh.change_dir(&config.workspace_root);

        let mut args: Vec<String> = vec![self.subcommand.clone()];

        // Package
        if let Some(ref pkg) = self.package {
            args.push("-p".into());
            args.push(pkg.clone());
        }

        // Target resolution: if a JSON spec exists use it, otherwise treat as built-in target
        let target_spec = config.target_spec(&self.target);
        if target_spec.exists() {
            let spec_path = target_spec
                .to_str()
                .context("Target spec path is not valid UTF-8")?;
            args.push("--target".into());
            args.push(spec_path.to_string());
            args.push("-Zjson-target-spec".into());
        } else {
            args.push("--target".into());
            args.push(self.target.clone());
        }

        // build-std flags (always needed for kernel targets)
        args.push("-Zbuild-std=core,compiler_builtins,alloc".into());
        args.push("-Zbuild-std-features=compiler-builtins-mem".into());

        if self.release {
            args.push("--release".into());
        }

        // Extra args after --
        if !self.extra_args.is_empty() {
            args.push("--".into());
            args.extend(self.extra_args.clone());
        }

        let mut command = cmd!(sh, "cargo {args...}");
        if let Some(ref profile) = self.profile {
            command = command.env("CARGO_IMAGE_RUNNER_PROFILE", profile);
        }
        command
            .run()
            .with_context(|| format!("cargo {} failed", self.subcommand))?;

        Ok(())
    }
}
