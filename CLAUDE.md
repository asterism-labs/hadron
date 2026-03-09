# Hadron

Hadron is an x86_64 kernel written in Rust. The kernel follows a two-crate model: a monolithic **hadron-kernel** (arch primitives, driver API traits, memory management, scheduler, VFS, PCI core, device registry) and a pluggable **hadron-drivers** (hardware drivers registered via linker sections). Both run in ring 0.

## Build Commands

A `justfile` at the project root wraps `gluon` (auto-bootstrapped on first run). Run `just vendor` and `just configure` before the first build.

```sh
just build              # Build sysroot + all crates + kernel + initrd
just run [-- args]      # Build + run in QEMU
just test               # Run all tests (host + kernel)
just test --host-only   # Host-side unit tests only (fast)
just test --kernel-only # Kernel integration tests only (QEMU)
just check              # Type-check without linking
just clippy             # Run clippy lints on project crates
just fmt                # Format source files
just fmt --check        # Check formatting (CI)
just bench              # Run kernel benchmarks
just configure          # Resolve config + generate rust-project.json
just menuconfig         # TUI configuration editor
just vendor             # Fetch/sync vendored dependencies
just clean              # Remove build artifacts
just miri               # Miri tests on hadron-core sync primitives
just loom               # Loom concurrency tests on hadron-core
```

Global flags: `--profile <name>` (`-P`), `--target <triple>`, `--verbose` (`-v`), `--force` (`-f`).

Build configuration lives in `gluon.rhai` (Rhai scripting) — defines targets, config options, profiles, crate groups, and QEMU settings.

## File Reading

Source files in this project can be large (500-2000+ lines). Always use bounded reads:

- Use `Read` with `offset` and `limit` to fetch only the section you need plus ~20 lines of context.
- Use `Grep` first to locate line numbers, then `Read` the range — never read an entire large file to find a single function.
- In Bash, use `head -n` or `tail -n` — never unbounded `cat`.
- Files in `vendor/`, `target/`, and `build/` should almost never be read directly. Use `Grep` to locate specific lines first.

## Required Skills

Before performing any of these actions, invoke the corresponding skill. Do not rely on memory of prior invocations — invoke the skill each session.

| Action | Skill |
|--------|-------|
| Creating commits, branching, rebasing, PRs | `hadron-git-workflow` |
| Writing or modifying Rust code | `hadron-code-standards` |
| Exploring architecture, boot flow, testing | `hadron-architecture` |

## Quality Gates

Before every `git commit`, run these checks. Delegate each to a **Task subagent** (`Bash` type) to keep the main context clean:

1. **Format check**: `just fmt --check` — fix with `just fmt` if needed, then re-stage
2. **Lint check**: `just clippy` — fix all warnings (pedantic clippy)
3. **Host tests**: `just test --host-only` — must pass (fast, no QEMU)

Run these in parallel as Task subagents. Only proceed to `git commit` after all three pass. If a check fails, fix the issue and re-run only the failing check.

The `validate-commit-msg.sh` hook only validates message format, not code quality. Quality is the agent's responsibility.

## Commit Messages

Always invoke the `hadron-git-workflow` skill before committing. Key rules:

- Format: `type(scope): summary` — imperative, lowercase, no period, max 72 chars
- Body must have changelog sections (`### Added`, `### Changed`, `### Fixed`, `### Removed`)
- No `Co-Authored-By` trailers — scope is required on every commit
- Only fast-forward merges — no merge commits

See the `hadron-git-workflow` skill for the full scope taxonomy and examples.

## Subagent Delegation

Use Task subagents (`Bash` type) to offload long-running work from the main context:

- **Quality gates**: Delegate `just fmt --check`, `just clippy`, and `just test --host-only` to parallel subagents before committing.
- **Build verification**: Delegate `just build` when verifying the full build after significant changes.
- **Kernel tests**: Delegate `just test --kernel-only` (runs QEMU, takes 30+ seconds) to a background subagent.

When running build tools directly in the main context (not via subagent), keep output short to protect context space:

- Pipe through `tail -20` to see only the summary: `just clippy 2>&1 | tail -20`
- Use `just check` (type-check only) for quick feedback before a full `just build`
- Prefer subagent delegation over direct execution for any command with verbose output

## Known Issues

See [`docs/src/reference/known-issues.md`](docs/src/reference/known-issues.md) for tracked bugs, limitations, and the lock ordering reference table.
