## Context
Where do learning records live on disk?

- **Workspace state** (`.orbit/state/learnings/`, gitignored). Same locality as job runs, command audit, etc. Workspace-private; doesn't survive collaborator handoff.
- **Workspace-scoped, checked in** (`.orbit/learnings/<id>.yaml`, in git). Same locality as tasks. Travels with the repo across machines and collaborators.
- **Global** (`~/.orbit/learnings/`). Like the global skills location. Cross-workspace; requires conflict semantics if multiple workspaces author overlapping records.

Per the Scoping Rules table in [CLAUDE.md](../../../CLAUDE.md), tasks are `WorkspaceOnly` and live in `.orbit/tasks/` checked in. Job runs are also `WorkspaceOnly` but under `.orbit/state/`, gitignored, because they're execution artifacts. Learnings sit closer to tasks in shape — durable project artifacts authored over time — so the task locality is the right precedent.

The cross-workspace case ([3_vision.md §1.4](./3_vision.md)) is real but secondary: most learnings are repo-specific, and the cross-cutting ones are best handled by tag-driven promotion later, not by making the default storage location global.

## Decision
Phase 1 stores learnings at `.orbit/learnings/<id>.yaml`, scoped `WorkspaceOnly` per the Scoping Rules table, checked into git. The SQLite index lives under `.orbit/state/` and is rebuildable from the YAML; it does not need to be checked in.

## Consequences
- Learnings travel with the repo. New collaborator clones, gets all the project knowledge from day zero.
- A learning authored on one machine and a task fix on another arrive in the same PR and review together, which keeps the knowledge in lockstep with the code that produced it.
- The git semantics for tasks (review, merge, conflict resolution) apply uniformly; no new mental model needed.
- Cost: every learning is a commit. PR diffs include learning records, which is fine for substantive learnings but adds review noise for housekeeping edits (typo fixes, scope-glob tweaks). Merge conflicts on the SQLite index are avoided by gitignoring it, but conflicts on the YAML are possible when two PRs add learnings simultaneously — handled by ID allocation (date + sequence), but worth noting.

---
