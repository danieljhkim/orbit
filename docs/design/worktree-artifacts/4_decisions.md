---
summary: "Worktree Artifacts - Decisions"
type: design
title: "Worktree Artifacts - Decisions"
owner: codex
last_updated: 2026-05-20
status: Accepted
feature: worktree-artifacts
doc_role: decisions
tags: ["worktree-artifacts"]
paths: ["crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-cli/**"]
related_features: ["worktree-artifacts"]
related_artifacts: ["ORB-00199", "ORB-00200", "ORB-00201", "ADR-0177"]
---

# Worktree Artifacts - Decisions

ADR-style log for worktree artifact storage. Entries use globally allocated ADR IDs; the corresponding `.orbit/adrs/...` artifact is the source of truth for lifecycle metadata.

## ADR-0177 - Worktree-local ADR and learning bodies with shared ID allocation

**Status:** Accepted - 2026-05 - [ORB-00201]

**Context.** Linked worktrees need ADR and learning bodies committed with the code branch that created them, but IDs must remain collision-free across all worktrees. [ORB-00199] introduced shared/local root resolution and [ORB-00200] introduced the shared SQLite allocator. The remaining choice was whether body files follow the allocator into `shared_root` or follow the editing branch into `local_root`.

**Decision.** Write ADR and learning body files under the current worktree `local_root` while keeping ID allocation, migration, and allocation metadata in `shared_root/.orbit/state/semantic.db`. Lists read through `id_allocations`: default output includes only locally readable bodies, while `include_remote` returns stubs that name the recorded worktree and branch.

**Consequences.**
- ADR and learning files can be staged in the same PR as the implementation that created them.
- Shared ID allocation still prevents cross-worktree collisions and records where each body lives.
- Readers get predictable defaults without failing on missing sibling-worktree files.
- Cost: list/show paths now carry a federation boundary and must handle `body_path` metadata, remote stubs, and stale worktree paths.

## Task References

- [ORB-00199] introduced shared/local root resolution.
- [ORB-00200] introduced shared ID allocation and `L-NNNN`.
- [ORB-00201] implemented this decision.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
