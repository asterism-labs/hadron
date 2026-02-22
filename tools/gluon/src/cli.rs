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

    /// Suppress per-crate output; show only errors and the final summary.
    #[arg(long, short = 'q', global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Enable verbose output with timing and cache diagnostics.
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,

    /// Maximum number of parallel workers (0 or omitted = auto-detect from CPU count).
    #[arg(long, short = 'j', global = true)]
    pub jobs: Option<usize>,
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
    /// Analyze profiling data captured from kernel serial output.
    Perf(PerfArgs),
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

    /// Only run kernel-internal (ktest) tests.
    #[arg(long)]
    pub ktest_only: bool,

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

/// Arguments for the `perf` subcommand.
#[derive(Parser)]
pub struct PerfArgs {
    /// Perf subcommand to execute.
    #[command(subcommand)]
    pub command: PerfCommand,
}

/// Perf analysis subcommands.
#[derive(Subcommand)]
pub enum PerfCommand {
    /// Analyze HPRF profiling data and generate reports.
    Report(PerfReportArgs),
    /// Run kernel in QEMU with serial capture for profiling data collection.
    Record(PerfRecordArgs),
}

/// Arguments for `perf report`.
#[derive(Parser)]
pub struct PerfReportArgs {
    /// Path to captured serial binary containing HPRF data.
    #[arg(long)]
    pub input: String,

    /// Path to kernel ELF binary for symbol resolution.
    #[arg(long)]
    pub kernel: String,

    /// Report mode: flat (default), flamegraph, or folded.
    #[arg(long, default_value = "flat")]
    pub mode: String,

    /// Output file path (required for flamegraph and folded modes).
    #[arg(short = 'o', long)]
    pub output: Option<String>,
}

/// Arguments for `perf record`.
#[derive(Parser)]
pub struct PerfRecordArgs {
    /// Path to kernel binary to profile.
    #[arg(long)]
    pub kernel: String,

    /// Output path for captured serial data (default: build/profile_serial.bin).
    #[arg(short = 'o', long)]
    pub output: Option<String>,

    /// Extra arguments passed to QEMU after `--`.
    #[arg(last = true)]
    pub extra_args: Vec<String>,
}
