# Build System

Hadron uses a custom build tool (`gluon`) that invokes `rustc` directly instead of using Cargo for kernel compilation. This gives full control over sysroot construction, target configuration, and cross-compilation without relying on nightly-only Cargo flags.

## Quick Start

```sh
# First-time setup
just vendor             # Fetch/sync vendored dependencies
just configure          # Resolve config + generate rust-project.json

# Common commands
just build              # Build the kernel
just run                # Build + launch in QEMU
just test               # Run all tests
just test --host-only   # Host unit tests only (fast)
just check              # Type-check without linking
just clippy             # Lint project crates
just fmt                # Format source files
just fmt --check        # Check formatting (CI)
just menuconfig         # TUI configuration editor
just vendor             # Fetch/sync vendored dependencies
just clean              # Remove build/ directory
```

Global flags:
- `--profile <name>` or `-P <name>` — select a build profile from `gluon.rhai`
- `--target <triple>` — override the target
- `--verbose` (`-v`) — verbose output
- `--force` (`-f`) — force rebuild

## Configuration Files

### `hadron.toml`

Master configuration at the project root. Defines targets, config options, build profiles, QEMU settings, and test configuration.

**Targets** map a triple name to a JSON target spec and optional linker script:

```toml
[targets.x86_64-unknown-hadron]
spec = "targets/x86_64-unknown-hadron.json"
linker-script = "targets/x86_64-unknown-hadron.ld"
```

**Config options** are Kconfig-style typed values that become compile-time constants and `--cfg` flags:

```toml
[config.options.serial_log]
type = "bool"        # bool, u32, u64, str
default = true
depends-on = []      # error if this is true but a dependency is false
select = []          # enabling this auto-enables these options
help = "Enable serial port logging"
```

Bool options become `--cfg hadron_<name>` flags on kernel crates, and all options are compiled into a `hadron_config` crate as `pub const` values.

**Profiles** provide named build configurations with inheritance:

```toml
[profiles.default]
target = "x86_64-unknown-hadron"
opt-level = 0
debug-info = true
boot-binary = "hadron-boot-limine"

[profiles.stress]
inherits = "default"
config = { smp = true, MAX_CPUS = 4 }
```

### `crates.toml`

The crate registry defines every compilation unit, its dependencies, and its context. This is the file you edit when adding or removing crates.

Each entry is defined in `gluon.rhai` as part of a crate group. Key fields include path, edition, type (`lib`, `bin`, `proc-macro`), context (`sysroot`, `host`, `userspace`, or kernel by default), dependencies, and features.

**Contexts** determine how a crate is compiled:

| Context | Target | Sysroot | Description |
|---------|--------|---------|-------------|
| `sysroot` | kernel target | builds it | core, compiler_builtins, alloc |
| `host` | host triple | system | proc-macros and their dependencies |
| *(absent)* | kernel target | custom | kernel crates (the default) |
| `userspace` | userspace target | custom | userspace binaries |

## Adding a New Crate

### Adding a kernel library crate

1. Create the crate directory (e.g. `crates/my-crate/src/lib.rs`)
2. Add it to `crates.toml`:

```toml
[crate.my-crate]
path = "crates/my-crate"
edition = "2024"
deps = { hadron_kernel = "hadron-kernel" }  # dependencies
```

3. Add it as a dependency of whatever crate uses it:

```toml
[crate.hadron-kernel]
deps = { ..., my_crate = "my-crate" }
```

4. If it has host-runnable unit tests, add it to `hadron.toml`:

```toml
[tests]
host-testable = [..., "my-crate"]
```

5. Run `just configure` to regenerate `rust-project.json`.

### Adding a vendored external crate

1. Place the source in `vendor/<name>/` (with its own `Cargo.toml` for host tests)
2. Add it to `crates.toml` with no `context` (kernel target):

```toml
[crate.some-lib]
path = "vendor/some-lib"
edition = "2021"
features = ["no_std_feature"]
```

3. Add it as a dependency where needed.

### Adding a proc-macro crate

1. Create the crate with `type = "proc-macro"` and `context = "host"`:

```toml
[crate.my-macros]
path = "crates/my-macros"
type = "proc-macro"
context = "host"
deps = { syn = "syn", quote = "quote", proc_macro2 = "proc-macro2" }
```

2. Reference it as a proc-macro dependency:

```toml
[crate.my-lib]
deps = { my_macros = { crate = "my-macros", proc-macro = true } }
```

### Adding a userspace binary

1. Create the crate with `type = "bin"` and `context = "userspace"`:

```toml
[crate.my-program]
path = "userspace/my-program"
type = "bin"
context = "userspace"
root = "src/main.rs"
deps = { lepton_syslib = "lepton-syslib" }
```

The binary will be automatically included in the initrd.

## Build Pipeline

When you run `just build`, the following steps execute in order:

1. **Sysroot** — Compile `core`, `compiler_builtins`, `alloc` for the kernel target
2. **Host crates** — Compile proc-macros and their deps for the host triple
3. **Config crate** — Generate and compile `hadron_config` (typed constants from `hadron.toml`)
4. **Kernel crates** — Compile all kernel crates in dependency order (topological sort)
5. **HBTF** — Generate backtrace symbol file from the kernel ELF
6. **Initrd** — Compile userspace binaries, package into CPIO archive

Build artifacts go to `build/`:

```
build/
├── sysroot/          # Custom sysroot (core, compiler_builtins, alloc)
├── host/             # Host-compiled proc-macros (.dylib/.so)
├── kernel/<target>/  # Kernel crate artifacts (.rlib, binary)
├── incremental/      # Rustc incremental compilation cache
├── generated/        # Generated sources (hadron_config.rs)
├── backtrace.hbtf    # Backtrace symbol file
└── initrd.cpio       # Userspace initrd archive
```

## rust-analyzer Support

Run `just configure` to generate `rust-project.json` at the project root. This gives rust-analyzer:

- Correct dependency graph for all 24+ crates
- Per-crate target (host triple for proc-macros, custom kernel target for kernel crates)
- Active `#[cfg(hadron_*)]` flags from the current profile
- Feature flags per crate
- Proc-macro dylib paths (for macro expansion in the IDE)

Re-run `just configure` after changing `crates.toml`, `hadron.toml`, or adding new crates.
