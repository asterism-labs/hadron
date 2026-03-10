---
name: hadron-git-workflow
description: Use when creating commits, branching, rebasing, merging, or creating PRs in the Hadron project
---

# Hadron Git Workflow

## Branching with Worktrees
- Always use `git worktree` to develop features in isolated directories
- Create a worktree per feature branch — do not switch branches in the main worktree
- Clean up worktrees after merging: `git worktree remove <path>`

## Pre-Commit Quality Gates

Before every `git commit`, all three checks must pass:

1. `just fmt --check` — formatting is correct (fix with `just fmt` if needed)
2. `just clippy` — no lint warnings (pedantic clippy)
3. `just test --host-only` — host-side unit tests pass

Delegate these to parallel Task subagents to keep the main context clean.
Do not commit until all three pass. Re-run only the failing check after fixes.

## Commit Message Format

```
type(scope): short summary (imperative, lowercase, no period, <=72 chars)

Optional paragraph explaining motivation/approach.

### Added
- New capability or feature

### Changed
- Modification to existing behavior

### Fixed
- Bug fix

### Removed
- Removed feature or deprecated code

BREAKING CHANGE: description of what breaks (only when applicable)
```

## Types

`feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`

## Scope Taxonomy

Scope is **required** on every commit. Use the most specific scope that covers the primary change. Cross-cutting changes use the parent scope.

### Kernel
`hadron-kernel`, `hadron-kernel/arch`, `hadron-kernel/boot`, `hadron-kernel/hw`

### Crates
`hadron-core`, `hadron-objects`, `hadron-mm`, `hadron-sched`, `hadron-pci`, `hadron-mmio`, `hadron-intrinsics`, `hadron-acpi`, `hadron-elf`, `hadron-dwarf`, `hadron-binparse`, `hadron-fdt`, `hadron-linkset`, `hadron-log`, `hadron-boot-info`, `hadron-bench`, `hadron-test`, `hadron-ktest`, `hadron-utest`

### Userspace
`lepton-syslib`, `hadron-libc`

### Tools
`gluon`, `gluon/vendor`, `gluon/config`, `hadron-perf`, `hadron-runner`

### Other
`workspace`, `boot/uefi`, `docs`, `ci`

## Rules

1. **Scope is required** on every commit
2. Use the **most specific scope** that covers the primary change
3. Cross-cutting changes use the parent scope (e.g., `hadron-kernel` instead of `hadron-kernel/mm`)
4. Body must contain **at least one changelog section** (`### Added`, `### Changed`, `### Fixed`, `### Removed`)
   - Trivial `chore`/`style` commits are exempt from this rule
5. **No `Co-Authored-By` trailers**
6. Subject line: imperative mood, lowercase, no trailing period, max 72 characters

## Examples

```
feat(hadron-kernel/mm): add physical memory allocator

### Added
- Bitmap-based physical frame allocator
- Allocation stats via sys_query
```

```
fix(hadron-kernel/arch): correct off-by-one in page table walk

### Fixed
- Page table walk returned wrong entry for addresses at PML4 boundary
```

```
refactor(hadron-kernel/arch): extract GDT setup into dedicated module

### Changed
- GDT initialization moved from boot.rs to gdt.rs
- Per-CPU GDT reload now handled by cpu::init()
```

```
feat(gluon): add build caching and parallel compilation

### Added
- Content-hash based build cache
- Parallel rustc invocation across independent crates

### Changed
- Default build profile now enables incremental compilation
```

## Merge Strategy
- Always prefer fast-forward merges or rebasing — no merge commits
- Rebase feature branches onto `main` before merging: `git rebase main`
- Merge with: `git merge --ff-only <branch>`
- If conflicts arise during rebase, resolve them incrementally per commit
