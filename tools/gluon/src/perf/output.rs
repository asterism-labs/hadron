//! Terminal output formatting for benchmark and profiling results.

use super::bench_analysis;
use super::wire::HBenchResults;

/// Print benchmark results as a formatted table.
pub fn print_bench_table(results: &HBenchResults) {
    let stats = bench_analysis::compute_stats(results);
    let freq = results.tsc_freq_khz;

    if stats.is_empty() {
        println!("  No benchmark results to display.");
        return;
    }

    // Compute column widths.
    let max_name = stats
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    // Header.
    println!();
    println!(
        "  {:<width$}  {:>12}  {:>12}  {:>12}  {:>12}  {:>8}",
        "Name",
        "Median (cy)",
        "Mean (cy)",
        "Min (cy)",
        "Stddev (cy)",
        "Samples",
        width = max_name
    );
    println!(
        "  {:-<width$}  {:->12}  {:->12}  {:->12}  {:->12}  {:->8}",
        "",
        "",
        "",
        "",
        "",
        "",
        width = max_name
    );

    for stat in &stats {
        println!(
            "  {:<width$}  {:>12}  {:>12}  {:>12}  {:>12}  {:>8}",
            stat.name,
            stat.median,
            stat.mean,
            stat.min,
            stat.stddev,
            stat.count,
            width = max_name
        );
    }

    // Summary with nanosecond conversion.
    if freq > 0 {
        println!();
        println!("  TSC frequency: {} kHz", freq);
        println!(
            "  Total elapsed: {} ns ({:.3} ms)",
            results.total_nanos,
            results.total_nanos as f64 / 1_000_000.0
        );
    }
    println!();
}

/// Print a flat profiling report from sample data.
pub fn print_flat_profile(
    entries: &[(String, u64)],
    total_samples: u64,
) {
    if entries.is_empty() {
        println!("  No profiling samples to display.");
        return;
    }

    let max_name = entries
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(8)
        .max(8);

    println!();
    println!(
        "  {:<width$}  {:>8}  {:>6}",
        "Function",
        "Samples",
        "%",
        width = max_name
    );
    println!(
        "  {:-<width$}  {:->8}  {:->6}",
        "",
        "",
        "",
        width = max_name
    );

    for (name, count) in entries {
        let pct = if total_samples > 0 {
            (*count * 100) / total_samples
        } else {
            0
        };
        println!(
            "  {:<width$}  {:>8}  {:>5}%",
            name,
            count,
            pct,
            width = max_name
        );
    }
    println!();
}
