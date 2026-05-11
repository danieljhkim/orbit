## Context
The original layout used `.orbit/knowledge/graph/refs/current.json` — one mutable ref shared across every branch and worktree. The last rebuild won globally. Multi-branch and multi-worktree workflows therefore saw graph reads for the wrong branch, and concurrent rebuilds raced on the single pointer.

## Decision
Namespace refs by branch: `refs/heads/<branch>.json`. Reads resolve the current git branch; writes fail on detached HEAD rather than invent a label. Reads fall back to the default branch's ref with a stderr warning when the current-branch ref does not yet exist; writes never fall back.

## Consequences
- Two worktrees on different branches can rebuild concurrently without corruption.
- A new branch remains readable via direct fallback, while auto-refresh materializes the current branch ref before treating the graph as fresh.
- Migration path: legacy `refs/current.json` is moved to `refs/heads/<default>.json` on open.
- Cost: two worktrees on the *same* branch still share a ref (see [2_design.md §6.5]).

---
