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

A `justfile` at the project root wraps `gluon` (auto-bootstrapped on first run). Run `just vendor` and `just configure` before the first build.

```sh
just build              # Build sysroot + all crates + kernel + initrd
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
just clean              # Remove build artifacts
```

Global flags: `--profile <name>` (`-P`), `--target <triple>`, `--verbose` (`-v`), `--force` (`-f`).

Build configuration lives in `gluon.rhai` (Rhai scripting) — defines targets, config options, profiles, crate groups, and QEMU settings.

## Development Phases

Phases 0-7 are complete (boot, serial, CPU init, PMM, VMM, interrupts, async executor, syscalls). The 8 remaining phases are documented in `docs/src/phases/`. Phase 6 introduced an async cooperative executor, so all remaining phases are designed around the async model (async VFS, async block devices, per-CPU executors for SMP). See `docs/src/SUMMARY.md` for the full phase listing.
