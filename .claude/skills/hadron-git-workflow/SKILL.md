---
name: hadron-git-workflow
description: Use when creating commits, branching, rebasing, merging, or creating PRs in the Hadron project
---

# Hadron Git Workflow

## Branching with Worktrees
- Always use `git worktree` to develop features in isolated directories
- Create a worktree per feature branch — do not switch branches in the main worktree
- Clean up worktrees after merging: `git worktree remove <path>`

## Commit Messages
- Follow [Conventional Commits](https://www.conventionalcommits.org/) format
- Prefixes: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`
- Format: `<prefix>: <short summary>` (imperative mood, lowercase, no period, max ~72 chars)
- Optional body: one blank line after summary, then a short description if needed
- Do NOT include `Co-Authored-By` trailers
- Examples:
  - `feat: add physical memory allocator`
  - `fix: correct off-by-one in page table walk`
  - `refactor: extract GDT setup into dedicated module`

## Merge Strategy
- Always prefer fast-forward merges or rebasing — no merge commits
- Rebase feature branches onto `main` before merging: `git rebase main`
- Merge with: `git merge --ff-only <branch>`
- If conflicts arise during rebase, resolve them incrementally per commit
