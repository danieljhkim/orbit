## Context
The history walker diffs every new commit against its first parent, parses unified diffs, and resolves trees. An in-process git library (`gix`, `git2`) would avoid per-commit fork cost.

## Decision
Shell out to `git` via `orbit_common::utility::git::run_git`. No in-process git dependency.

## Consequences
- Behavior matches what a user sees on the command line — trivially reproducible.
- No linked library adds build surface or ABI risk.
- Cost: per-commit fork overhead. Tolerable at current repo sizes; revisit if it shows up in refresh profiles.

---
