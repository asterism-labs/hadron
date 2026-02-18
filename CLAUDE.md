# Hadron

Hadron is an x86_64 kernel written in Rust following the **framekernel** architecture. The kernel is split into two layers: an unsafe **frame** (`hadron-core`) that directly interacts with hardware and exports safe abstractions, and safe **services** (`hadron-kernel`) that implement high-level functionality using only safe Rust. Both layers run in ring 0 with zero IPC overhead.

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
├── xtask/                  # Build automation (host-side)
├── targets/                # Custom target specs (x86_64-unknown-hadron.json)
├── docs/                   # mdbook documentation
└── limine.conf             # Limine bootloader config
```

## Build Commands

All build commands use the xtask pattern (`cargo xtask <command>`):

- `cargo xtask build` — Cross-compile kernel for `x86_64-unknown-hadron` target
- `cargo xtask run` — Build + create ISO + launch QEMU
- `cargo xtask test` — Build + run tests in QEMU (exit code 33 = success, uses isa-debug-exit)

## Formatting & Linting

Recipes are in the `justfile` (requires [`just`](https://github.com/casey/just)):

- `just fmt` — Format all source files (cargo fmt + taplo fmt)
- `just fmt-check` — Check formatting without modifying files
- `just lint` — Run all lints (clippy + taplo check + typos)
- `just check` — Format then lint (one-stop command)
- `just build` / `just run` / `just test` — Delegates to `cargo xtask`

External tools: `just`, `taplo`, `typos-cli` (install with `brew install just taplo typos-cli`).

## Architecture

### Boot Flow

```
Limine bootloader → kernel/boot/limine (hadron-boot-limine)
    → hadron_kernel::kernel_init(boot_info)
        → GDT, IDT, PMM, VMM initialization
        → kernel main loop
```

## Code Quality Standards

- **Unsafe discipline:** All `unsafe` blocks require `// SAFETY:` comments. Never cast `&T` to `*mut T` — use interior mutability. `hadron-kernel` must minimize unsafe; each block must justify why it can't live in `hadron-core`.
- **Globals:** Only kernel-wide singletons (allocator, executor, logger) as module-level statics. Consolidate related atomics into locked structs. `SpinLock<Option<T>>` for init-once globals with descriptive panic messages.
- **DRY:** Inline asm in exactly ONE canonical location in `hadron-core::arch::<arch>::instructions` or `registers`. Named constants for all hardware values — no magic numbers.
- **RAII:** Resources with cleanup must implement `Drop`. No manual `destroy()` methods.
- **Docs:** All public items require `///` doc comments. All modules require `//!` module-level docs. Complex bit manipulation and safety invariants need inline comments.
- **Errors:** Typed error enums, not `&'static str`. `expect()` messages describe the invariant violated. Propagate with `?`; `unwrap()` only for bug-indicating invariants.
- **Organization:** Arch-specific code under `arch/<arch>/`. One concern per module.
- **Dead code:** Targeted `#[allow(dead_code)]` with a comment referencing the future phase (e.g., `// Phase 6: used by scheduler`). No blanket crate-level suppression.
- **Hardware abstraction:** Key subsystems define traits (`InterruptController`, `ClockSource`, `Timer`). Traits in `hadron-driver-api`, implementations in `hadron-drivers`.

## Git Workflow

### Branching with Worktrees
- Always use `git worktree` to develop features in isolated directories
- Create a worktree per feature branch — do not switch branches in the main worktree
- Clean up worktrees after merging: `git worktree remove <path>`

### Commit Messages
- Follow [Conventional Commits](https://www.conventionalcommits.org/) format
- Prefixes: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`
- Format: `<prefix>: <short summary>` (imperative mood, lowercase, no period, max ~72 chars)
- Do NOT include `Co-Authored-By` trailers

### Merge Strategy
- Always prefer fast-forward merges or rebasing — no merge commits
- Rebase feature branches onto `main` before merging: `git rebase main`
- Merge with: `git merge --ff-only <branch>`

## Testing

Integration tests run in QEMU using `hadron-test` crate:
- Tests use `isa-debug-exit` device (iobase=0xf4) to signal pass/fail
- Exit code 33 = success
- Timeout: 30 seconds per test
- Run with: `cargo xtask test`

## Development Phases

Phases 0-6 are complete; Phase 7 (Syscall Interface) is in progress. Phase 6 implemented an async cooperative executor, so all remaining phases use the async model. See `docs/src/SUMMARY.md` for the full phase listing.
