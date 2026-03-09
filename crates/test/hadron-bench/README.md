# hadron-bench

A `no_std` microbenchmark harness for Hadron kernel benchmarks, built on the same architecture as `hadron-test`. Benchmarks use fenced `RDTSC` cycle counting for measurement and emit results both as human-readable serial text and a compact binary wire format (HBENCH) for host-side analysis by the gluon build system or `hadron-perf`.

## Features

- Custom benchmark runner compatible with `#![test_runner(hadron_bench::bench_runner)]` and `#[test_case]` attributes
- `Bencher` iteration controller with configurable warmup and sample counts, using `LFENCE`-fenced `RDTSC` for cycle-accurate timing
- `bench_entry_point!` macro for minimal Limine-booted CPU microbenchmarks
- `bench_entry_point_with_init!` macro for benchmarks requiring full kernel initialization (PMM, VMM, heap)
- Integer-only statistics (min, max, median, mean, stddev) computed without floating point, suitable for kernel environments
- Binary wire format emission (HBENCH) over serial with per-benchmark raw cycle samples, TSC frequency, and total elapsed time
- Command-line argument parsing with filter, `--exact`, `--list`, `--quiet`, `--warmup N`, `--samples N`, and `--skip NAME`
- `black_box` optimization barrier to prevent the compiler from eliding benchmark workloads
