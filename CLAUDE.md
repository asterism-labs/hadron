# Hadron

Hadron is an x86_64 kernel written in Rust following the **framekernel** architecture. The kernel is split into two layers: an unsafe **frame** (`hadron-core`) that directly interacts with hardware and exports safe abstractions, and safe **services** (`hadron-kernel`) that implement high-level functionality using only safe Rust. Both layers run in ring 0 with zero IPC overhead.

## Naming Convention

The project uses a particle physics naming theme:
- **Hadron** (kernel) — kernel crates keep the `hadron-` prefix
- **Gluon** (build system) — the build tool at `tools/gluon/`; gluons bind quarks into hadrons
- **Lepton** (userspace) — userspace crates use the `lepton-` prefix; leptons are a separate particle family

## Project Structure

```
hadron/
├── kernel/
│   ├── hadron-core/        # Frame: unsafe core, safe public API (depends on bitflags)
│   ├── hadron-kernel/      # Services: arch init, drivers, mm (depends on hadron-core, hadron-drivers, noalloc)
│   └── boot/
│       └── limine/         # Limine boot stub binary (hadron-boot-limine)
├── crates/
│   ├── limine/             # Limine boot protocol bindings
│   ├── noalloc/            # Allocation-free data structures
│   ├── hadron-drivers/     # Hardware drivers
│   ├── hadron-test/        # Test framework (QEMU isa-debug-exit)
│   └── uefi/               # UEFI bindings (Phase 2)
├── userspace/
│   ├── init/               # Init process (first userspace binary, lepton-init)
│   ├── lepton-syslib/      # Userspace syscall library
│   ├── shell/              # Interactive shell (lepton-shell)
│   ├── spinner/            # CPU-bound preemption demo (lepton-spinner)
│   ├── spawn-worker/       # Worker child for stress tests (lepton-spawn-worker)
│   ├── spawn-stress/       # Spawn stress test (lepton-spawn-stress)
│   ├── pipe-consumer/      # Pipe consumer child (lepton-pipe-consumer)
│   └── pipe-test/          # Pipe test orchestrator (lepton-pipe-test)
├── tools/
│   └── gluon/              # Custom build system (invokes rustc directly)
├── targets/                # Custom target specs (x86_64-unknown-hadron.json)
├── vendor/                 # Vendored external dependencies
├── gluon.rhai              # Build configuration script (targets, crates, profiles, pipeline)
├── docs/                   # mdbook documentation
└── limine.conf             # Limine bootloader config
```

## Build Commands

The project uses `gluon`, a custom build tool at `tools/gluon/` that invokes `rustc` directly. It builds a custom sysroot, compiles all crates in dependency order, and provides Kconfig-like configuration via `gluon.rhai` (Rhai scripting).

First, build the tool itself:
```sh
cargo build --manifest-path tools/gluon/Cargo.toml
```

Then use it (from the project root):
```sh
gluon configure        # Resolve config + generate rust-project.json
gluon build            # Build sysroot + all crates + kernel + HBTF + initrd
gluon run [-- args]    # Build + run in QEMU via cargo-image-runner
gluon test             # Run all tests (host + kernel)
gluon test --host-only # Run host-side unit tests only
gluon check            # Type-check kernel crates without linking
gluon clippy           # Run clippy lints on project crates
gluon fmt              # Format project source files
gluon fmt --check      # Check formatting without modifying
gluon clean            # Remove build artifacts
```

Global flags: `--profile <name>` (`-P`) selects a build profile from `gluon.rhai`, `--target <triple>` overrides the target.

### Configuration

Build configuration lives in `gluon.rhai` at the project root, using the Rhai scripting language. The script defines:
- **Project metadata**: name and version
- **Targets**: custom target specs and linker scripts
- **Config options**: typed kernel options (bool, u32, u64, str) with dependencies, selects, ranges, and choices
- **Profiles**: named configurations (default, release, stress, debug-gdb) with inheritance
- **Groups**: crate collections with shared compilation context (sysroot, host, kernel, userspace)
- **Rules**: artifact generation (HBTF, initrd) with built-in or script handlers
- **Pipeline**: ordered build stages with barriers and rule execution
- **QEMU settings**: machine type, memory, extra args, test exit codes

The build system evaluates `gluon.rhai` to produce a `BuildModel`, validates it, resolves configuration, then schedules and executes compilation stages.

## Architecture

### Boot Flow

```
Limine bootloader → kernel/boot/limine (hadron-boot-limine)
    → hadron_kernel::kernel_init(boot_info)
        → GDT, IDT, PMM, VMM initialization
        → kernel main loop
```

### Key Dependencies

- `hadron-core` depends on `bitflags`
- `hadron-kernel` depends on `hadron-core`, `hadron-drivers`, `noalloc`
- Boot stub depends on `hadron-core`, `hadron-kernel`, `hadron-drivers`, `limine`, `noalloc`
- All `crates/*` are standalone no_std libraries

### Custom Target

The kernel uses a custom target `x86_64-unknown-hadron` (not `x86_64-unknown-none`):
- Kernel code model, PIC relocation
- Soft-float (no SSE/AVX in kernel mode)
- Panic = abort, redzone disabled
- Uses `rust-lld` linker

## Lint Configuration

Clippy lints are applied by `gluon clippy` to project crates (kernel/, crates/, userspace/) using `-W clippy::all -W clippy::pedantic`. Vendored crates are type-checked but not linted.

### Dead Code Annotation Policy

- Use targeted `#[allow(dead_code)]` on specific items, NOT blanket crate-level suppression
- Each annotation must include a comment referencing the future phase or purpose:
  ```rust
  #[allow(dead_code)] // Phase 6: used by scheduler
  ```
- Public APIs in library crates don't need annotation (pub items don't trigger dead_code)
- Review and remove annotations when the referenced phase is implemented

## Code Quality Standards

### Unsafe Discipline
- All `unsafe` blocks require a `// SAFETY:` comment explaining the invariant being upheld
- Never cast `&T` to `*mut T` for mutation — use interior mutability (`UnsafeCell`, `Cell`, or locks)
- Prefer safe wrappers: if a struct guarantees preconditions at construction, expose safe methods internally calling unsafe
- `hadron-kernel` (safe services layer) must minimize unsafe — each `unsafe` block must justify why it can't live in `hadron-core`

### Globals Policy
- Only truly kernel-wide singletons may be module-level statics: global allocator, executor, logger
- Related atomics must be consolidated into a single locked struct — no scattering of 3+ related `AtomicU64`s across a module
- Prefer passing subsystem handles through function arguments over `with_*` global accessors
- `SpinLock<Option<T>>` for init-once globals is acceptable but accessor functions must panic with descriptive messages

### DRY / No Duplication
- Inline asm for any instruction must exist in exactly ONE canonical location in `hadron-core::arch::<arch>::instructions` or `registers`
- Extract repeated patterns into helper functions at 3+ call sites
- Named constants for all hardware/arch values — never bare `4096`, `0xFFF`, `0x10`, etc.

### RAII & Ownership
- Resources with cleanup (mapped pages, physical frames, MMIO regions) must implement `Drop`
- No manual `destroy()` methods — use `Drop` or scoped guard patterns
- Hardware driver structs holding MMIO mappings own their mapping lifetime

### Documentation
- All public items require `///` doc comments (enforced by `missing_docs = "warn"`)
- All modules require `//!` module-level docs explaining purpose and key types
- Complex bit manipulation, register layouts, and safety invariants require inline comments

### Error Handling
- Use typed error enums, not `&'static str`
- `expect()` messages describe the invariant violated, not just "failed to X"
- Propagate errors with `?` where possible; `expect()`/`unwrap()` only for invariants indicating bugs

### Code Organization
- Arch-specific code belongs under `arch/<arch>/`, not at crate root
- Large constant data (fonts, tables) in dedicated files
- One concern per module

### Hardware Abstraction
- Key subsystems define traits: `InterruptController`, `ClockSource`, `Timer`
- Traits in `hadron-driver-api`, implementations in `hadron-drivers`
- Access hardware through trait interfaces where feasible

## Git Workflow

### Branching with Worktrees
- Always use `git worktree` to develop features in isolated directories
- Create a worktree per feature branch — do not switch branches in the main worktree
- Clean up worktrees after merging: `git worktree remove <path>`

### Commit Messages
- Follow [Conventional Commits](https://www.conventionalcommits.org/) format
- Prefixes: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`
- Format: `<prefix>: <short summary>` (imperative mood, lowercase, no period, max ~72 chars)
- Optional body: one blank line after summary, then a short description if needed
- Do NOT include `Co-Authored-By` trailers
- Examples:
  - `feat: add physical memory allocator`
  - `fix: correct off-by-one in page table walk`
  - `refactor: extract GDT setup into dedicated module`

### Merge Strategy
- Always prefer fast-forward merges or rebasing — no merge commits
- Rebase feature branches onto `main` before merging: `git rebase main`
- Merge with: `git merge --ff-only <branch>`
- If conflicts arise during rebase, resolve them incrementally per commit

## Testing

- `gluon test --host-only` — Run host unit tests for crates listed in `gluon.rhai` `tests().host_testable()`
- `gluon test --kernel-only` — Build kernel + run integration tests in QEMU
- `gluon test` — Run both host and kernel tests

Integration tests run in QEMU using `hadron-test` crate:
- Tests use `isa-debug-exit` device (iobase=0xf4) to signal pass/fail
- Exit code 33 = success (configured in `gluon.rhai` `qemu()` section)
- Timeout: 30 seconds per test

## Development Phases

The project has a completed foundation (Phases 0-6) plus 9 remaining phases documented in `docs/src/phases/`. Phases 0-6 are complete; Phase 7 (Syscall Interface) is in progress. Phase 6 implemented an async cooperative executor instead of the originally-planned preemptive scheduler, so all remaining phases are designed around the async model (async VFS, async block devices, per-CPU executors for SMP). See `docs/src/SUMMARY.md` for the full phase listing.
