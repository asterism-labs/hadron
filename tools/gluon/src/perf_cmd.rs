//! CLI entry point for `gluon perf` subcommands.
//!
//! Bridges the `perf` analysis modules (wire parser, symbol resolver,
//! profile analysis, flamegraph) to the command-line interface.

use std::path::Path;

use anyhow::{Result, bail};

use crate::cli::{PerfArgs, PerfCommand, PerfRecordArgs, PerfReportArgs};
use crate::config::ResolvedConfig;
use crate::perf;
use crate::run;

/// Dispatch `gluon perf <subcommand>`.
pub fn cmd_perf(args: &PerfArgs, config: Option<&ResolvedConfig>) -> Result<()> {
    match &args.command {
        PerfCommand::Report(report_args) => cmd_perf_report(report_args),
        PerfCommand::Record(record_args) => cmd_perf_record(record_args, config),
    }
}

/// `gluon perf report` — parse HPRF data and produce a profile report.
fn cmd_perf_report(args: &PerfReportArgs) -> Result<()> {
    let input_path = Path::new(&args.input);
    let kernel_path = Path::new(&args.kernel);

    // Read the captured serial data.
    let serial_data = std::fs::read(input_path)
        .map_err(|e| anyhow::anyhow!("failed to read input '{}': {e}", input_path.display()))?;

    println!(
        "Parsing HPRF data from {} ({} bytes)...",
        input_path.display(),
        serial_data.len()
    );

    let results = perf::wire::parse_hprf(&serial_data)?;

    println!(
        "  {} samples, {} ftrace entries, {} CPUs",
        results.samples.len(),
        results.ftrace_entries.len(),
        results.cpu_count
    );

    // Build symbol resolver from kernel ELF.
    let resolver = perf::symbol_resolver::SymbolResolver::from_kernel_elf(
        kernel_path,
        results.kernel_vbase,
    );

    match args.mode.as_str() {
        "flat" => {
            let (entries, total) = perf::profile_analysis::flat_profile(&results, &resolver);
            perf::output::print_flat_profile(&entries, total);
        }
        "flamegraph" => {
            let output = args.output.as_deref().ok_or_else(|| {
                anyhow::anyhow!("flamegraph mode requires --output (-o) path for SVG file")
            })?;
            let output_path = Path::new(output);

            // Write folded stacks to a temp file, then generate SVG.
            let folded_path = output_path.with_extension("folded");
            perf::flamegraph::write_folded_stacks(&results, &resolver, &folded_path)?;
            perf::flamegraph::write_flamegraph_svg(&folded_path, output_path)?;
        }
        "folded" => {
            let output = args.output.as_deref().ok_or_else(|| {
                anyhow::anyhow!("folded mode requires --output (-o) path")
            })?;
            perf::flamegraph::write_folded_stacks(&results, &resolver, Path::new(output))?;
        }
        other => {
            bail!("unknown report mode '{other}' (expected: flat, flamegraph, folded)");
        }
    }

    Ok(())
}

/// `gluon perf record` — run kernel in QEMU and capture serial profiling data.
fn cmd_perf_record(args: &PerfRecordArgs, config: Option<&ResolvedConfig>) -> Result<()> {
    let config = config.ok_or_else(|| {
        anyhow::anyhow!("perf record requires a resolved build configuration")
    })?;

    let kernel_path = Path::new(&args.kernel);
    if !kernel_path.exists() {
        bail!("kernel binary not found: {}", kernel_path.display());
    }

    let output_path = match &args.output {
        Some(p) => std::path::PathBuf::from(p),
        None => config.root.join("build/profile_serial.bin"),
    };

    println!(
        "Recording profiling data from {}...",
        kernel_path.display()
    );

    let (result, serial_data) =
        run::run_with_serial_capture(config, kernel_path, &args.extra_args)?;

    // Save raw serial bytes.
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_path, &serial_data)
        .map_err(|e| anyhow::anyhow!("failed to write output '{}': {e}", output_path.display()))?;

    // Summary.
    println!("\n  Captured {} bytes to {}", serial_data.len(), output_path.display());

    // Check for HPRF magic as a quick sanity check.
    if serial_data.windows(4).any(|w| w == b"HPRF") {
        println!("  HPRF data detected in capture.");
    } else {
        println!("  Note: no HPRF magic found (kernel may not have profiling enabled).");
    }

    if result.timed_out {
        bail!("QEMU timed out during profiling capture");
    }
    if !result.success {
        bail!(
            "QEMU exited with code {} (expected success)",
            result.exit_code
        );
    }

    Ok(())
}
