//! CLI entry point for `gluon perf` subcommands.
//!
//! Bridges the `perf` analysis modules (wire parser, symbol resolver,
//! profile analysis, flamegraph) to the command-line interface.

use std::path::Path;

use anyhow::{Result, bail};

use crate::cli::{PerfArgs, PerfCommand, PerfReportArgs};
use crate::perf;

/// Dispatch `gluon perf <subcommand>`.
pub fn cmd_perf(args: &PerfArgs) -> Result<()> {
    match &args.command {
        PerfCommand::Report(report_args) => cmd_perf_report(report_args),
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
