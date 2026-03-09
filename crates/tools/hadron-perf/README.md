# hadron-perf

A host-side performance analysis library for Hadron kernel benchmarks and profiling data. It deserializes the binary formats emitted by the kernel's benchmark harness (`hadron-bench`) and sampling profiler, then provides statistical analysis, baseline comparison, symbol resolution, flame graph generation, and formatted terminal output.

## Features

- Parses the HBENCH binary wire format to extract per-benchmark raw cycle samples, TSC frequency, and timing metadata
- Parses the HPRF binary wire format to extract CPU sample records with full stack traces and ftrace entries
- Computes benchmark statistics (min, max, median, mean, stddev) and prints formatted result tables
- Saves and compares JSON baseline files with configurable regression threshold detection
- Resolves virtual addresses to demangled function names using the kernel ELF symbol table (via `hadron-elf`)
- Generates folded stack traces and SVG flame graphs (via `inferno`) from profiling sample data
- Produces flat profile reports showing per-function sample counts and percentages
