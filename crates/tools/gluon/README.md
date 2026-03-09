# gluon

Gluon is the custom build system for the Hadron kernel project. Named after the particle that binds quarks into hadrons, it replaces Cargo with a standalone tool that invokes `rustc` directly, compiles a custom sysroot (`core`, `alloc`, `compiler_builtins`) for bare-metal targets, and orchestrates the full build pipeline from configuration through compilation to bootable ISO generation. Build configuration is defined declaratively in `gluon.rhai` using the Rhai scripting language, which specifies targets, crate groups, profiles, config options, dependency declarations, pipeline stages, and QEMU settings.

## Features

- **Direct rustc invocation** -- bypasses Cargo entirely, assembling rustc flags per-crate with precise control over `--extern`, `--edition`, `--cfg`, sysroot paths, and linker scripts
- **Rhai-scripted configuration** -- the build model (targets, crates, groups, profiles, config options, pipeline, QEMU, bootloader) is declared in `gluon.rhai` with a fluent builder API; supports `include` for splitting config across files
- **Custom sysroot compilation** -- builds `core`, `compiler_builtins`, and `alloc` from rustc source for each custom target triple, cached and reused across builds
- **Parallel DAG scheduler** -- constructs a global dependency DAG across all pipeline stages and compiles crates in parallel using a thread-pool, allowing host proc-macros, sysroot builds, and cross-compiled crates to overlap
- **Incremental build cache** -- tracks compiler flags, source file timestamps, and SHA-256 content hashes per crate; uses rustc `.d` dep-info files for precise invalidation with hybrid mtime/hash fallback
- **Dependency vendoring** -- fetches crates from crates.io or git repositories into `vendor/`, resolves transitive dependencies, generates a `gluon.lock` lockfile, and supports `--check` verification and `--prune` cleanup
- **Kconfig-style configuration** -- typed config options (bool, u32, u64, string, choice, list) with `depends_on`, `selects`, range constraints, and menu grouping; interactive TUI editor via `menuconfig` (built on ratatui)
- **Artifact generation** -- produces initrd archives (CPIO), bootable ISO images (via xorriso), HBTF backtrace tables, and HKIF kernel info files
- **QEMU integration** -- `gluon run` builds and launches the kernel in QEMU with configurable machine type, memory, core count, and extra arguments
- **Testing and benchmarking** -- host-side unit tests, QEMU-based kernel integration tests with timeout and exit-code validation, and kernel benchmarks with baseline comparison and regression detection
- **Profiling support** -- `perf record` captures serial profiling data from QEMU, `perf report` analyzes HPRF data with symbol resolution via DWARF, producing flat reports or folded stacks for flamegraphs
- **rust-project.json generation** -- `configure` produces a rust-analyzer project file so IDE features work across all kernel and userspace crates
- **Source formatting** -- `fmt` runs rustfmt on project crates with `--check` support for CI

## Architecture

Gluon is structured as a pipeline that flows from script evaluation to artifact output:

- **`engine`** -- Rhai scripting engine that evaluates `gluon.rhai` and populates a `BuildModel` through registered builder types (target, profile, group, crate, rule, pipeline, dependency, config, etc.)
- **`model`** -- pure data types representing the complete build model (no Rhai dependencies), serializable for caching
- **`validate`** -- checks the build model for consistency (missing targets, circular deps, unknown references)
- **`config`** -- resolves a `BuildModel` into a `ResolvedConfig` by applying profile inheritance, config option dependencies, and target resolution
- **`kconfig`** -- lexer/parser for Kconfig DSL files, producing typed config option definitions with menu structure
- **`crate_graph`** -- resolves crate dependency edges from the build model into a topologically ordered compilation graph
- **`scheduler`** -- builds a global DAG of all compilation units and executes them in parallel with a thread-pool worker model
- **`compile`** -- assembles rustc command lines per-crate and invokes the compiler, tracking output artifacts for downstream extern linking
- **`sysroot`** -- compiles `core`, `compiler_builtins`, and `alloc` from the rustc sysroot source for custom targets
- **`cache`** -- build cache manifest using mtime + SHA-256 freshness checks with rustc dep-info integration
- **`vendor`** -- fetches, resolves transitive dependencies, and manages the `vendor/` directory and `gluon.lock` lockfile
- **`artifact`** -- post-compilation artifact generators: initrd (CPIO), ISO images, HBTF backtrace tables, HKIF kernel info
- **`run`** -- launches QEMU with the built kernel and configured options
- **`test` / `bench`** -- compiles and runs kernel test/benchmark binaries in QEMU with result parsing
- **`menuconfig`** -- interactive TUI configuration editor built on ratatui/crossterm
- **`analyzer`** -- generates `rust-project.json` for rust-analyzer IDE support
- **`fmt`** -- source formatting via rustfmt
- **`perf` / `perf_cmd`** -- profiling data analysis and QEMU serial capture
