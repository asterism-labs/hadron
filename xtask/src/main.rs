//! Build automation for Hadron kernel.
//!
//! Usage:
//!   cargo xtask build    - Build kernel
//!   cargo xtask run      - Build and run in QEMU (via cargo-image-runner)
//!   cargo xtask test     - Run all tests (host unit tests + QEMU integration)
//!   cargo xtask test --host-only        - Run only host-side unit tests
//!   cargo xtask test --integration-only - Run only QEMU integration tests
//!   cargo xtask codegen  - Run code generators (fonts, etc.)
//!   cargo xtask check    - Type-check kernel code
//!   cargo xtask clippy   - Run clippy lints on kernel code
//!   cargo xtask doc      - Generate documentation

mod build;
mod cargo;
mod codegen;
mod config;
mod hbtf;
mod initrd;
mod test;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::cargo::CargoCommand;
use crate::config::Config;

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Build automation for Hadron kernel")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build the kernel
    Build {
        /// Build in release mode
        #[arg(short, long)]
        release: bool,

        /// Target triple (default: from workspace metadata)
        #[arg(short, long)]
        target: Option<String>,

        /// Package to build (default: hadron-boot-limine)
        #[arg(short, long)]
        package: Option<String>,
    },

    /// Build and run in QEMU (via cargo-image-runner)
    Run {
        /// Build in release mode
        #[arg(short, long)]
        release: bool,

        /// Target triple (default: from workspace metadata)
        #[arg(short, long)]
        target: Option<String>,

        /// Image-runner profile preset (stress, debug, minimal)
        #[arg(long)]
        profile: Option<String>,

        /// Extra arguments passed after --
        #[arg(last = true)]
        extra_args: Vec<String>,
    },

    /// Run tests (host unit tests + QEMU integration tests)
    Test {
        /// Build in release mode
        #[arg(short, long)]
        release: bool,

        /// Target triple (default: from workspace metadata)
        #[arg(short, long)]
        target: Option<String>,

        /// Package to test (default: hadron-kernel, integration only)
        #[arg(short, long)]
        package: Option<String>,

        /// Image-runner profile preset (stress, debug, minimal)
        #[arg(long)]
        profile: Option<String>,

        /// Run only host-side unit tests (skip QEMU integration tests)
        #[arg(long, conflicts_with = "integration_only")]
        host_only: bool,

        /// Run only QEMU integration tests (skip host unit tests)
        #[arg(long, conflicts_with = "host_only")]
        integration_only: bool,

        /// Extra arguments passed after -- (forwarded to test binary)
        #[arg(last = true)]
        extra_args: Vec<String>,
    },

    /// Type-check kernel code without full compilation
    Check {
        /// Target triple (default: from workspace metadata)
        #[arg(short, long)]
        target: Option<String>,

        /// Package to check (default: hadron-boot-limine)
        #[arg(short, long)]
        package: Option<String>,
    },

    /// Run clippy lints on kernel code
    Clippy {
        /// Target triple (default: from workspace metadata)
        #[arg(short, long)]
        target: Option<String>,

        /// Package to lint (default: hadron-boot-limine)
        #[arg(short, long)]
        package: Option<String>,
    },

    /// Run code generators (fonts, etc.) from codegen.toml
    Codegen,

    /// Generate documentation for kernel crates
    Doc {
        /// Target triple (default: from workspace metadata)
        #[arg(short, long)]
        target: Option<String>,

        /// Open documentation in browser after building
        #[arg(long)]
        open: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;

    match cli.command {
        Commands::Build {
            release,
            target,
            package,
        } => {
            let target = target.unwrap_or_else(|| config.default_target.clone());
            initrd::build_initrd(&config.workspace_root)?;
            let result = build::build(&config, &target, package.as_deref(), release)?;
            hbtf::generate_hbtf(
                &result.kernel_binary,
                &config.target_dir.join("backtrace.hbtf"),
                !release,
            )?;
            println!("Built: {}", result.kernel_binary.display());
        }

        Commands::Run {
            release,
            target,
            profile,
            extra_args,
        } => {
            let target = target.unwrap_or_else(|| config.default_target.clone());
            initrd::build_initrd(&config.workspace_root)?;
            let result = build::build(&config, &target, Some("hadron-boot-limine"), release)?;
            hbtf::generate_hbtf(
                &result.kernel_binary,
                &config.target_dir.join("backtrace.hbtf"),
                !release,
            )?;
            CargoCommand {
                subcommand: "run".into(),
                target,
                package: Some("hadron-boot-limine".into()),
                release,
                extra_args,
                profile,
            }
            .run(&config)?;
        }

        Commands::Test {
            release,
            target,
            package,
            profile,
            host_only,
            integration_only,
            extra_args,
        } => {
            let target = target.unwrap_or_else(|| config.default_target.clone());
            if !host_only {
                initrd::build_initrd(&config.workspace_root)?;
                let result = build::build(&config, &target, Some("hadron-boot-limine"), release)?;
                hbtf::generate_hbtf(
                    &result.kernel_binary,
                    &config.target_dir.join("backtrace.hbtf"),
                    !release,
                )?;
            }
            test::run_tests(
                &config,
                &target,
                package.as_deref(),
                release,
                profile.as_deref(),
                host_only,
                integration_only,
                extra_args,
            )?;
        }

        Commands::Codegen => {
            codegen::run_codegen(&config.workspace_root)?;
        }

        Commands::Check { target, package } => {
            let target = target.unwrap_or_else(|| config.default_target.clone());
            let package = package.unwrap_or_else(|| "hadron-boot-limine".into());
            CargoCommand {
                subcommand: "check".into(),
                target,
                package: Some(package),
                release: false,
                extra_args: vec![],
                profile: None,
            }
            .run(&config)?;
        }

        Commands::Clippy { target, package } => {
            let target = target.unwrap_or_else(|| config.default_target.clone());
            let package = package.unwrap_or_else(|| "hadron-boot-limine".into());
            CargoCommand {
                subcommand: "clippy".into(),
                target,
                package: Some(package),
                release: false,
                extra_args: vec![],
                profile: None,
            }
            .run(&config)?;
        }

        Commands::Doc { target, open } => {
            let target = target.unwrap_or_else(|| config.default_target.clone());

            // doc uses --workspace --exclude instead of -p, so we build args manually
            let sh = xshell::Shell::new()?;
            sh.change_dir(&config.workspace_root);

            let target_spec = config.target_spec(&target);
            let mut args: Vec<String> = vec![
                "doc".into(),
                "--workspace".into(),
                "--exclude".into(),
                "xtask".into(),
            ];

            if target_spec.exists() {
                let spec_path = target_spec
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Target spec path is not valid UTF-8"))?;
                args.push("--target".into());
                args.push(spec_path.to_string());
                args.push("-Zjson-target-spec".into());
            } else {
                args.push("--target".into());
                args.push(target.clone());
            }

            args.push("-Zbuild-std=core,compiler_builtins,alloc".into());
            args.push("-Zbuild-std-features=compiler-builtins-mem".into());

            if open {
                args.push("--open".into());
            }

            xshell::cmd!(sh, "cargo {args...}")
                .run()
                .map_err(|e| anyhow::anyhow!("cargo doc failed: {e}"))?;
        }
    }

    Ok(())
}
