## Context
`orbit-knowledge` was built as a git-like history layer: content-addressed objects, mutable refs, atomic swaps, lock protocols. In practice the graph is consumed as "fresh queryable index of the current code" — none of the version-store affordances are used by agents.

## Decision
Reframe the graph as a derived index, regenerable from `(file_contents, extractor_version)`. Delete object storage, mutable refs, and atomic-swap locking. Single SQLite file per worktree is the only durable state.

## Consequences
- Deletes ~3k LOC of object-store, lock, and ref-management code.
- Removes the lock protocol's structural inability to coordinate same-branch worktrees (see knowledge-graph ADR-002 cost line).
- Cost: **no history.** "What did the graph look like at commit X?" is no longer a query the graph can answer. Use git for that.