---
name: hadron-code-standards
description: Use when writing, reviewing, or modifying Rust code in the Hadron kernel project
---

# Hadron Code Standards

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
