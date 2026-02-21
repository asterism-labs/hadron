//! Command-line interface definitions for gluon.

use clap::{Parser, Subcommand};

/// Hadron kernel build system.
#[derive(Parser)]
#[command(name = "gluon", version, about)]
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

    /// Enable verbose output with timing and cache diagnostics.
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,
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
    /// Interactive TUI configuration editor.
    Menuconfig,
    /// Remove build artifacts.
    Clean,
    /// Run kernel benchmarks.
    Bench(BenchArgs),
    /// Vendor external dependencies into vendor/.
    Vendor(VendorArgs),
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

/// Arguments for the `bench` subcommand.
#[derive(Parser)]
pub struct BenchArgs {
    /// Filter benchmarks by name substring.
    #[arg(long)]
    pub filter: Option<String>,

    /// Save benchmark results as a baseline JSON file.
    #[arg(long)]
    pub save_baseline: Option<String>,

    /// Compare against a baseline JSON file and flag regressions.
    #[arg(long)]
    pub baseline: Option<String>,

    /// Regression threshold as a percentage (default: 5).
    #[arg(long, default_value = "5")]
    pub threshold: u32,

    /// Output format for benchmark results.
    #[arg(long, default_value = "table")]
    pub format: String,

    /// Extra arguments passed to the benchmark harness after `--`.
    #[arg(last = true)]
    pub extra_args: Vec<String>,
}

/// Arguments for the `vendor` subcommand.
#[derive(Parser)]
pub struct VendorArgs {
    /// Verify lock file and vendor directory without modifying anything.
    #[arg(long)]
    pub check: bool,

    /// Remove vendored crates no longer referenced by any dependency.
    #[arg(long)]
    pub prune: bool,
}

/// Arguments for the `fmt` subcommand.
#[derive(Parser)]
pub struct FmtArgs {
    /// Check formatting without modifying files.
    #[arg(long)]
    pub check: bool,
}
