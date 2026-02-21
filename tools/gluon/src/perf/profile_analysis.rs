//! Profiling data analysis: flat profiles from sample data.

use std::collections::HashMap;

use super::symbol_resolver::SymbolResolver;
use super::wire::HPrfResults;

/// Aggregate samples into a flat profile (function -> count).
///
/// Uses the top-of-stack address from each sample as the function attribution.
/// Returns entries sorted by count (descending).
pub fn flat_profile(
    results: &HPrfResults,
    resolver: &SymbolResolver,
) -> (Vec<(String, u64)>, u64) {
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;

    for sample in &results.samples {
        if let Some(&addr) = sample.stack.first() {
            let name = resolver
                .resolve(addr)
                .unwrap_or_else(|| format!("{addr:#018x}"));
            *counts.entry(name).or_default() += 1;
            total += 1;
        }
    }

    let mut entries: Vec<(String, u64)> = counts.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));

    (entries, total)
}

/// Generate folded stack traces for flame graph generation.
///
/// Each line is: `func1;func2;func3 count\n`
/// where func1 is the bottom of the stack and func3 is the top.
pub fn folded_stacks(
    results: &HPrfResults,
    resolver: &SymbolResolver,
) -> String {
    let mut stacks: HashMap<String, u64> = HashMap::new();

    for sample in &results.samples {
        if sample.stack.is_empty() {
            continue;
        }

        // Build the stack string (bottom-up: reverse of captured order).
        let stack_str: String = sample
            .stack
            .iter()
            .rev()
            .map(|&addr| {
                resolver
                    .resolve(addr)
                    .unwrap_or_else(|| format!("{addr:#x}"))
            })
            .collect::<Vec<_>>()
            .join(";");

        *stacks.entry(stack_str).or_default() += 1;
    }

    let mut output = String::new();
    let mut entries: Vec<(&String, &u64)> = stacks.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));

    for (stack, count) in entries {
        output.push_str(stack);
        output.push(' ');
        output.push_str(&count.to_string());
        output.push('\n');
    }

    output
}
