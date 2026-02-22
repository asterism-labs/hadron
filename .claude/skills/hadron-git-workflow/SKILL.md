---
name: hadron-git-workflow
description: Use when creating commits, branching, rebasing, merging, or creating PRs in the Hadron project
---

# Hadron Git Workflow

## Branching with Worktrees
- Always use `git worktree` to develop features in isolated directories
- Create a worktree per feature branch — do not switch branches in the main worktree
- Clean up worktrees after merging: `git worktree remove <path>`

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
`hadron-kernel`, `hadron-kernel/arch`, `hadron-kernel/mm`, `hadron-kernel/sched`, `hadron-kernel/fs`, `hadron-kernel/ipc`, `hadron-kernel/proc`, `hadron-kernel/syscall`, `hadron-kernel/pci`, `hadron-kernel/driver_api`, `hadron-kernel/profiling`, `hadron-kernel/boot`

### Drivers
`hadron-drivers`, `hadron-drivers/ahci`, `hadron-drivers/virtio`, `hadron-drivers/serial`, `hadron-drivers/input`, `hadron-drivers/display`, `hadron-drivers/timer`, `hadron-drivers/pci`, `hadron-drivers/fs`

### Crates
`hadron-acpi`, `hadron-elf`, `hadron-dwarf`, `hadron-test`, `hadron-core`, `hadron-bench`, `hadron-codegen`, `hadron-binparse`, `hadron-mmio`, `hadron-syscall`, `hadron-driver-macros`, `planck-noalloc`, `linkset`, `limine`

### Userspace
`lepton-init`, `lepton-shell`, `lepton-syslib`, `lepton-coreutils`

### Tools
`gluon`, `gluon/vendor`, `gluon/config`

### Other
`boot/limine`, `docs`, `ci`

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
refactor(hadron-drivers): extract GDT setup into dedicated module

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
