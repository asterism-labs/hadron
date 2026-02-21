//! Benchmark statistical analysis and baseline comparison.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::wire::HBenchResults;

/// Statistics for a single benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchStat {
    /// Benchmark name.
    pub name: String,
    /// Minimum cycles.
    pub min: u64,
    /// Maximum cycles.
    pub max: u64,
    /// Median cycles.
    pub median: u64,
    /// Mean cycles.
    pub mean: u64,
    /// Standard deviation (integer approximation).
    pub stddev: u64,
    /// Number of samples.
    pub count: usize,
}

/// A baseline file containing benchmark statistics and metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct Baseline {
    /// TSC frequency in kHz at time of baseline.
    pub tsc_freq_khz: u64,
    /// Benchmark statistics keyed by name.
    pub benchmarks: HashMap<String, BenchStat>,
}

/// Compute statistics for all benchmark records.
pub fn compute_stats(results: &HBenchResults) -> Vec<BenchStat> {
    results
        .records
        .iter()
        .filter_map(|record| {
            let mut samples = record.samples.clone();
            let n = samples.len();
            if n == 0 {
                return None;
            }

            samples.sort_unstable();

            let min = samples[0];
            let max = samples[n - 1];
            let median = if n % 2 == 0 {
                (samples[n / 2 - 1] + samples[n / 2]) / 2
            } else {
                samples[n / 2]
            };
            let sum: u128 = samples.iter().map(|&s| u128::from(s)).sum();
            let mean = (sum / n as u128) as u64;
            let variance = if n > 1 {
                let var_sum: u128 = samples
                    .iter()
                    .map(|&s| {
                        let diff = if s >= mean { s - mean } else { mean - s };
                        u128::from(diff) * u128::from(diff)
                    })
                    .sum();
                (var_sum / (n as u128 - 1)) as u64
            } else {
                0
            };
            let stddev = isqrt(variance);

            Some(BenchStat {
                name: record.name.clone(),
                min,
                max,
                median,
                mean,
                stddev,
                count: n,
            })
        })
        .collect()
}

/// Save benchmark results as a JSON baseline file.
pub fn save_baseline(results: &HBenchResults, path: &str) -> Result<()> {
    let stats = compute_stats(results);
    let mut benchmarks = HashMap::new();
    for stat in stats {
        benchmarks.insert(stat.name.clone(), stat);
    }

    let baseline = Baseline {
        tsc_freq_khz: results.tsc_freq_khz,
        benchmarks,
    };

    let json =
        serde_json::to_string_pretty(&baseline).context("serializing baseline to JSON")?;
    std::fs::write(path, json).with_context(|| format!("writing baseline to {path}"))?;

    Ok(())
}

/// Compare benchmark results against a baseline and flag regressions.
pub fn compare_baseline(results: &HBenchResults, path: &str, threshold_pct: u32) -> Result<()> {
    let json = std::fs::read_to_string(path).with_context(|| format!("reading baseline {path}"))?;
    let baseline: Baseline =
        serde_json::from_str(&json).context("parsing baseline JSON")?;

    let current_stats = compute_stats(results);
    let mut regressions = 0;

    println!("\n  Baseline comparison (threshold: {threshold_pct}%):");

    for stat in &current_stats {
        if let Some(base) = baseline.benchmarks.get(&stat.name) {
            let diff = if stat.median > base.median {
                stat.median - base.median
            } else {
                base.median - stat.median
            };

            let pct = if base.median > 0 {
                (diff * 100) / base.median
            } else {
                0
            };

            let direction = if stat.median > base.median {
                "slower"
            } else {
                "faster"
            };

            let flag = if stat.median > base.median && pct > u64::from(threshold_pct) {
                regressions += 1;
                " REGRESSION"
            } else {
                ""
            };

            println!(
                "    {} : {} -> {} ({pct}% {direction}){flag}",
                stat.name, base.median, stat.median
            );
        } else {
            println!("    {} : new (no baseline)", stat.name);
        }
    }

    if regressions > 0 {
        println!("\n  {regressions} regression(s) detected!");
    } else {
        println!("\n  No regressions detected.");
    }

    Ok(())
}

/// Integer square root via Newton's method.
fn isqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
