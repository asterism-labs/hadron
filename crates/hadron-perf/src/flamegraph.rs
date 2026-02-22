//! Flame graph generation from profiling data.
//!
//! Generates folded stacks format and optionally converts to SVG
//! using the `inferno` crate.

use anyhow::{Context, Result};
use std::io::BufRead;
use std::path::Path;

use crate::profile_analysis;
use crate::symbol_resolver::SymbolResolver;
use crate::wire::HPrfResults;

/// Generate a folded stacks file from profiling results.
pub fn write_folded_stacks(
    results: &HPrfResults,
    resolver: &SymbolResolver,
    output: &Path,
) -> Result<()> {
    let folded = profile_analysis::folded_stacks(results, resolver);
    std::fs::write(output, &folded)
        .with_context(|| format!("writing folded stacks to {}", output.display()))?;
    println!("  Folded stacks written to {}", output.display());
    Ok(())
}

/// Generate an SVG flame graph from folded stacks using inferno.
pub fn write_flamegraph_svg(folded_path: &Path, svg_path: &Path) -> Result<()> {
    let folded_data =
        std::fs::read_to_string(folded_path).with_context(|| format!("reading {}", folded_path.display()))?;

    let reader = std::io::BufReader::new(folded_data.as_bytes());
    let lines: Vec<String> = reader.lines().collect::<std::io::Result<_>>()?;

    let mut opts = inferno::flamegraph::Options::default();
    opts.title = "Hadron Kernel Profile".to_string();
    opts.count_name = "samples".to_string();

    let mut svg_output = Vec::new();
    inferno::flamegraph::from_lines(
        &mut opts,
        lines.iter().map(|s| s.as_str()),
        &mut svg_output,
    )
    .context("generating flame graph SVG")?;

    std::fs::write(svg_path, &svg_output)
        .with_context(|| format!("writing SVG to {}", svg_path.display()))?;

    println!("  Flame graph SVG written to {}", svg_path.display());
    Ok(())
}
