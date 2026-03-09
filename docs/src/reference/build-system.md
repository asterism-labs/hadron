# Build System

Hadron uses `gluon`, a Rhai-scripted build system that wraps `cargo` and provides kernel-specific features: custom target management, sysroot builds, Kconfig integration, QEMU launch, and vendor dependency resolution. `gluon` is auto-bootstrapped by `just` on first use — no manual installation is required.

## Quick Reference

```sh
just build              # Build sysroot + all crates + kernel image + initrd
just run [-- args]      # Build then launch in QEMU
just test               # Run all tests (host unit tests + QEMU kernel tests)
just test --host-only   # Host-side unit tests only (fast, no QEMU)
just test --kernel-only # Kernel integration tests only (requires QEMU)
just check              # Type-check all crates (no linking)
just clippy             # Run clippy lints on all project crates
just fmt                # Format all source files
just fmt --check        # Check formatting without modifying files (CI)
just bench              # Run kernel benchmarks
just configure          # Resolve Kconfig + generate rust-project.json for IDEs
just menuconfig         # Launch TUI Kconfig editor
just vendor             # Fetch and sync vendored dependencies
just clean              # Remove all build artifacts
just miri               # Run Miri on hadron-core sync primitives
just loom               # Run loom concurrency tests on sync primitives
```

Global flags available on all commands:

| Flag | Short | Meaning |
|------|-------|---------|
| `--profile <name>` | `-P` | Select build profile (default, release, stress, ...) |
| `--target <triple>` | | Override the default target triple |
| `--verbose` | `-v` | Show full cargo output |
| `--force` | `-f` | Force rebuild even if inputs are unchanged |

## gluon Build System

### Overview

`gluon` reads `gluon.rhai` at the project root and constructs a build model from the Rhai script. The model describes:
- Target triples and their JSON spec files.
- Build profiles with compiler settings and QEMU configuration.
- Crate groups (sysroot, kernel, userspace).
- External dependency versions (vendored into `vendor/`).
- Kconfig option declarations.

`gluon` then drives `cargo` with the correct flags for each crate group and handles post-build steps (link, initrd creation, image assembly).

### Bootstrap

`gluon` itself is a Rust binary. `just` checks for a cached binary at a known path and compiles it from source if absent. After the first `just` invocation completes, subsequent calls use the cached binary. Running `just vendor` also ensures `gluon`'s own dependencies are present.

### gluon.rhai Structure

The `gluon.rhai` file at the project root is evaluated by the Rhai interpreter embedded in `gluon`. It uses a chainable DSL:

```rhai
// Project metadata
project("hadron", "0.1.0");

// Register a custom target triple and its JSON spec
target("x86_64-unknown-hadron", "targets/x86_64-unknown-hadron.json");

// Load Kconfig declarations from a distributed Kconfig file tree
kconfig("Kconfig");

// Define a build profile
profile("default")
    .target("x86_64-unknown-hadron")
    .opt_level(0)
    .debug_info(true)
    .preset("debug")
    .qemu_cores(4)
    .boot_binary("hadron-boot-limine");

// Define a sysroot crate group (built before kernel crates)
let sysroot = group("sysroot")
    .target("x86_64-unknown-hadron")
    .edition("2024")
    .project(false);

sysroot.add("core", "{sysroot}/core").root("src/lib.rs");
sysroot.add("alloc", "{sysroot}/alloc").root("src/lib.rs")
    .deps(#{ core: "core", compiler_builtins: "compiler_builtins" });

// Register a vendored external dependency
dependency("bitflags").version("2.11.0");
```

### Profiles

| Profile | Description |
|---------|-------------|
| `default` | Debug build, opt-level 0, QEMU with 4 cores |
| `release` | Opt-level 2, thin LTO |
| `stress` | Like default, but with scheduler stress testing preset |
| `lock-stress` | Lock contention stress testing |
| `debug-gdb` | Adds `-s -S` to QEMU for GDB remote debugging |
| `debug-sanitizers` | Enables sanitizer presets |
| `profile` | Performance profiling preset |
| `net` | Adds QEMU TAP network device |

Select a profile with `just build -P release` or `just run -P debug-gdb`.

## Custom Target Triples

Hadron defines three custom Rust target specifications:

### x86_64-unknown-hadron (kernel)

Used for all kernel crates (`hadron-kernel`, `hadron-core`, `hadron-mm`, etc.):

| Setting | Value | Reason |
|---------|-------|--------|
| `os` | `none` | Bare metal, no OS |
| `rustc-abi` | `x86-softfloat` | No FPU in kernel (saves/restores are not set up) |
| `panic-strategy` | `abort` | No unwinding in kernel |
| `disable-redzone` | `true` | Interrupt handlers cannot assume 128-byte red zone |
| `code-model` | `kernel` | Kernel memory model for correct relocation handling |
| `relocation-model` | `pic` | Position-independent (for KASLR readiness) |
| `features` | `-mmx,-sse,...,+soft-float` | Disable all SIMD; enable soft-float |
| `has-thread-local` | `false` | No `#[thread_local]` (kernel uses CpuLocal instead) |
| `max-atomic-width` | `64` | Only 64-bit atomics guaranteed |

The `disable-redzone` setting is critical: without it, a hardware interrupt arriving during a kernel function that uses the red zone will corrupt the function's stack frame.

### x86_64-unknown-hadron-user (userspace)

Used for userspace programs and libraries compiled for Hadron. Enables the red zone, uses the standard `sysv64` calling convention, and links against `hadron-libc`.

### aarch64-unknown-hadron

The AArch64 kernel target for future porting work. Uses the `aarch64-unknown-none-softfloat` LLVM triple.

## Kconfig Integration

Hadron uses a distributed Kconfig system for kernel feature selection. The top-level `Kconfig` file includes subsystem-specific `Kconfig` files from crate directories.

### Configuration Options

```
CONFIG_MAX_CPUS=256        # Maximum supported CPUs
CONFIG_HADRON_LOCKDEP=y    # Enable lock dependency tracking
CONFIG_HADRON_LOCK_DEBUG=y # Enable lock nesting depth tracking
CONFIG_HADRON_LOCK_STAT=y  # Enable lock contention statistics
CONFIG_SMP=y               # Symmetric multiprocessing support
CONFIG_IOMMU=y             # VT-d IOMMU support
```

### How Kconfig Feeds Into Cargo

Gluon converts selected Kconfig options into `rustc` `--cfg` flags:

| Kconfig | Cargo cfg flag |
|---------|---------------|
| `CONFIG_HADRON_LOCKDEP=y` | `hadron_lockdep` |
| `CONFIG_HADRON_LOCK_DEBUG=y` | `hadron_lock_debug` |
| `CONFIG_HADRON_LOCK_STAT=y` | `hadron_lock_stat` |
| `CONFIG_SMP=y` | `hadron_smp` |

Kernel code uses `#[cfg(hadron_lockdep)]` to conditionally include lockdep logic. This keeps the non-debug kernel build free of overhead.

### menuconfig

`just menuconfig` launches a terminal UI (TUI) built with `ratatui` for interactive Kconfig editing. Changes are written to `.config` at the project root and picked up by the next `just configure` or `just build`.

## Vendor Dependencies

All external Rust crates are vendored into the `vendor/` directory. `just vendor` invokes `gluon vendor`, which:

1. Resolves all dependency versions from `gluon.rhai` `dependency()` declarations and `Cargo.toml` `[workspace.dependencies]`.
2. Downloads crate sources from crates.io (or a configured registry mirror).
3. Writes the resolved set to `gluon.lock`.
4. Configures `cargo` to use the `vendor/` directory exclusively (no network access during builds).

This ensures reproducible builds in offline environments and CI. The `vendor/` directory is committed to the repository.

## rust-project.json Generation

`just configure` generates `rust-project.json` for IDE support (rust-analyzer). This file describes all crates in the workspace, their source roots, edition, and dependencies — allowing rust-analyzer to provide accurate completions and diagnostics for `no_std` crates that cannot use the standard Cargo workspace discovery.

The generated `rust-project.json` is at the project root and is checked into the repository for convenience. It must be regenerated after adding new crates or changing Kconfig settings.

## Sysroot Build

The `core`, `alloc`, and `compiler_builtins` crates must be compiled for the custom `x86_64-unknown-hadron` target — the pre-compiled sysroot that ships with `rustup` targets a different ABI (it enables SSE, does not disable the red zone, etc.). `gluon` builds these from source as part of the `sysroot` crate group using the bundled sysroot sources in the `toolchain/` directory.

The `rust-toolchain.toml` at the project root pins the nightly toolchain version. Both the sysroot and the kernel crates are compiled with this same toolchain version.

## Test Infrastructure

### Host Unit Tests

`just test --host-only` compiles and runs tests for crates that support `cfg(test)` on the host (using the standard `x86_64-unknown-linux-gnu` target). These tests run in milliseconds.

Crates that are host-testable:
- `hadron-core` (sync primitives, CPU local storage, paging types)
- `hadron-mm` (page mapper, PMM algorithms)
- `hadron-acpi` (table parsing)
- `hadron-elf` (ELF parsing)

### Kernel Integration Tests

`just test --kernel-only` builds a test kernel image and launches it in QEMU. The test kernel uses the `hadron-ktest` framework to register test cases via linker section magic (`#[ktest]` attribute from `hadron-ktest-macros`). The kernel enumerates these test entries at boot, runs each test, and reports results to the serial port. `gluon` captures the serial output and parses pass/fail results.

These tests require QEMU and exercise code paths that depend on real hardware: interrupt delivery, SMP coordination, page table walks, ACPI parsing with actual BIOS tables.

### Loom and Miri

`just loom` rebuilds `hadron-core` with `cfg(loom)` and runs the loom test suite. This exhaustively model-checks all atomic interleavings in the sync primitives.

`just miri` runs `hadron-core`'s host tests under Miri, detecting undefined behavior in `unsafe` blocks.

Both are separate test modes from the normal `just test` suite and are run selectively on sync primitive changes.

## QEMU Configuration

The default profile launches QEMU with:
- 4 CPU cores (`-smp 4`)
- 512 MiB RAM
- Limine bootloader
- Serial output redirected to stdio
- KVM acceleration if available (falls back to software emulation)

QEMU arguments can be extended per-profile in `gluon.rhai`:

```rhai
profile("net")
    .inherits("default")
    .qemu_extra_args([
        "-netdev", "tap,id=net0,ifname=tap0,script=no,downscript=no",
        "-device", "virtio-net-pci,netdev=net0,mac=52:54:00:12:34:56"
    ]);
```

Additional QEMU arguments can be passed at the command line: `just run -- -monitor stdio`.
