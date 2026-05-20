## Context
Linked worktrees need ADR and learning bodies committed with the code branch that created them, but IDs must remain collision-free across all worktrees. ORB-00199 introduced shared/local root resolution and ORB-00200 introduced the shared SQLite allocator. The remaining choice is whether body files follow the allocator into shared_root or follow the editing branch into local_root.

## Decision
Write ADR and learning body files under the current worktree local_root while keeping ID allocation, migration, and allocation metadata in shared_root/.orbit/state/semantic.db. Lists read through id_allocations: default output includes only locally readable bodies, while include-remote returns stubs that name the recorded worktree and branch.

## Consequences
- ADR and learning files can be staged in the same PR as the implementation that created them.
- Shared ID allocation still prevents cross-worktree collisions and records where each body lives.
- Readers get predictable defaults without failing on missing sibling-worktree files.
- Cost: list/show paths now carry a federation boundary and must handle body_path metadata, remote stubs, and stale worktree paths.