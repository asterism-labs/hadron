//! Performance data analysis for benchmarks and profiling.
//!
//! Provides deserialization of HBENCH (benchmark) and HPRF (profiling) binary
//! formats, statistical analysis, baseline comparison, symbol resolution,
//! flame graph generation, and terminal output formatting.

pub mod bench_analysis;
pub mod flamegraph;
pub mod output;
pub mod profile_analysis;
pub mod symbol_resolver;
pub mod wire;
