//! Command-line interface definitions for hadron-build.

use clap::{Parser, Subcommand};

/// Hadron kernel build system.
#[derive(Parser)]
#[command(name = "hadron-build", version, about)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,

    /// Build profile to use.
    #[arg(long, short = 'P', global = true)]
    pub profile: Option<String>,

    /// Target triple (overrides profile default).
    #[arg(long, global = true)]
    pub target: Option<String>,

    /// Force rebuild, bypassing all cache checks.
    #[arg(long, short = 'f', global = true)]
    pub force: bool,
}

/// Available subcommands.
#[derive(Subcommand)]
pub enum Command {
    /// Compile the kernel and produce a bootable ISO.
    Build(BuildArgs),
    /// Build and run the kernel in QEMU.
    Run(RunArgs),
    /// Build and run tests.
    Test(TestArgs),
    /// Type-check without producing binaries.
    Check,
    /// Run clippy lints on project crates.
    Clippy,
    /// Format source files.
    Fmt(FmtArgs),
    /// Resolve configuration and generate rust-project.json.
    Configure,
    /// Remove build artifacts.
    Clean,
    /// Vendor external dependencies into vendor/.
    Vendor,
}

/// Arguments for the `build` subcommand.
#[derive(Parser)]
pub struct BuildArgs {
    /// Only build a specific crate (by name from crates.toml).
    #[arg(long, short = 'p')]
    pub package: Option<String>,
}

/// Arguments for the `run` subcommand.
#[derive(Parser)]
pub struct RunArgs {
    /// Extra arguments passed to QEMU after `--`.
    #[arg(last = true)]
    pub extra_args: Vec<String>,
}

/// Arguments for the `test` subcommand.
#[derive(Parser)]
pub struct TestArgs {
    /// Only run host-side unit tests.
    #[arg(long)]
    pub host_only: bool,

    /// Only run QEMU kernel integration tests.
    #[arg(long)]
    pub kernel_only: bool,

    /// Only run crash tests.
    #[arg(long)]
    pub crash_only: bool,

    /// Extra arguments passed to the test harness after `--`.
    #[arg(last = true)]
    pub extra_args: Vec<String>,
}

/// Arguments for the `fmt` subcommand.
#[derive(Parser)]
pub struct FmtArgs {
    /// Check formatting without modifying files.
    #[arg(long)]
    pub check: bool,
}
