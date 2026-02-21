# Hadron

Hadron is an x86_64 kernel written in Rust. The kernel follows a two-crate model: a monolithic **hadron-kernel** (arch primitives, driver API traits, memory management, scheduler, VFS, PCI core, device registry) and a pluggable **hadron-drivers** (hardware drivers registered via linker sections). Both run in ring 0.

## Naming Convention

The project uses a particle physics naming theme:
- **Hadron** (kernel) — kernel crates keep the `hadron-` prefix
- **Gluon** (build system) — the build tool at `tools/gluon/`; gluons bind quarks into hadrons
- **Lepton** (userspace) — userspace crates use the `lepton-` prefix; leptons are a separate particle family

## Project Structure

```
hadron/
├── kernel/
│   ├── hadron-kernel/      # Monolithic kernel: arch, driver API, mm, sched, VFS, PCI, device registry
│   ├── hadron-drivers/     # Pluggable drivers: AHCI, VirtIO, serial, display, input, FS impls
│   └── boot/
│       └── limine/         # Limine boot stub binary (hadron-boot-limine)
├── crates/
│   ├── limine/             # Limine boot protocol bindings
│   ├── noalloc/            # Allocation-free data structures
│   ├── hadron-test/        # Test framework (QEMU isa-debug-exit)
│   ├── acpi/               # ACPI table parsing (hadron-acpi)
│   ├── dwarf/              # DWARF debug info (hadron-dwarf)
│   ├── elf/                # ELF parser (hadron-elf)
│   └── uefi/               # UEFI bindings
├── userspace/
│   ├── init/               # Init process (lepton-init)
│   ├── lepton-syslib/      # Userspace syscall library
│   ├── shell/              # Interactive shell (lepton-shell)
│   └── coreutils/          # Core utilities (lepton-coreutils)
├── tools/
│   └── gluon/              # Custom build system (invokes rustc directly)
├── targets/                # Custom target specs (x86_64-unknown-hadron.json)
├── vendor/                 # Vendored external dependencies
├── gluon.rhai              # Build configuration script (targets, crates, profiles, pipeline)
├── docs/                   # mdbook documentation
└── limine.conf             # Limine bootloader config
```

## Build Commands

A `justfile` at the project root provides the primary build interface. All recipes auto-bootstrap `gluon` (the custom build tool) on first run.

### Prerequisites

```sh
just vendor             # Fetch/sync vendored dependencies
just configure          # Resolve config + generate rust-project.json
just build              # Build sysroot + all crates + kernel + initrd
```

### Common Commands

```sh
just build              # Build the kernel
just run [-- args]      # Build + run in QEMU
just test               # Run all tests (host + kernel)
just test --host-only   # Host-side unit tests only (fast)
just check              # Type-check without linking
just clippy             # Run clippy lints on project crates
just fmt                # Format source files
just fmt --check        # Check formatting (CI)
just configure          # Resolve config + generate rust-project.json
just menuconfig         # TUI configuration editor
just vendor             # Fetch/sync vendored dependencies
just vendor --check     # Verify vendor directory is up to date
just vendor --prune     # Remove unused vendored crates
just clean              # Remove build artifacts
```

### Global Flags

- `--profile <name>` (`-P`) — select a build profile from `gluon.rhai`
- `--target <triple>` — override the target
- `--verbose` (`-v`) — verbose output
- `--force` (`-f`) — force rebuild

### Configuration

Build configuration lives in `gluon.rhai` (Rhai scripting). It defines:
- Targets, custom target specs, and linker scripts
- Typed kernel config options (bool, u32, u64, str) with dependencies and selects
- Named profiles (default, release, stress, debug-gdb) with inheritance
- Crate groups, build pipeline stages, and QEMU settings

> **Note:** The justfile wraps `gluon`, a custom build tool at `tools/gluon/` that invokes `rustc` directly. For direct usage, see `tools/gluon/`.

## Architecture

### Boot Flow

```
Limine bootloader → kernel/boot/limine (hadron-boot-limine)
    → hadron_kernel::kernel_init(boot_info)
        → GDT, IDT, PMM, VMM initialization
        → kernel main loop
```

### Key Dependencies

- `hadron-kernel` depends on `bitflags`, `hadris-io`, `hadron-acpi`, `hadron-elf`, `hadron-syscall`, `noalloc`
- `hadron-drivers` depends on `hadron-kernel`, `bitflags`, `hadris-cpio`, `hadris-fat`, `hadris-io`, `hadris-iso`
- Boot stub depends on `hadron-kernel`, `hadron-drivers`, `limine`, `noalloc`
- Driver registration uses linker sections (`.hadron_pci_drivers`, `.hadron_platform_drivers`, `.hadron_block_fs`, etc.)
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
- `hadron-kernel` must minimize unsafe — each `unsafe` block must justify why it's needed

### Globals Policy
- Only truly kernel-wide singletons may be module-level statics: global allocator, executor, logger
- Related atomics must be consolidated into a single locked struct — no scattering of 3+ related `AtomicU64`s across a module
- Prefer passing subsystem handles through function arguments over `with_*` global accessors
- `SpinLock<Option<T>>` for init-once globals is acceptable but accessor functions must panic with descriptive messages

### DRY / No Duplication
- Inline asm for any instruction must exist in exactly ONE canonical location in `hadron_kernel::arch::<arch>::instructions` or `registers`
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
- Traits in `hadron_kernel::driver_api`, implementations in `hadron-drivers`
- Drivers register via linker-section macros (`pci_driver_entry!`, `platform_driver_entry!`, `block_fs_entry!`, etc.)
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

Phases 0-7 are complete (boot, serial, CPU init, PMM, VMM, interrupts, async executor, syscalls). The 8 remaining phases are documented in `docs/src/phases/`. Phase 6 introduced an async cooperative executor, so all remaining phases are designed around the async model (async VFS, async block devices, per-CPU executors for SMP). See `docs/src/SUMMARY.md` for the full phase listing.
