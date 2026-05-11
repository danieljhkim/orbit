## Context
Prototype graph mutation tools (`orbit.graph.add`, `orbit.graph.delete`, `orbit.graph.move`, and `orbit.graph.write`) implied that graph-node locks could coordinate write safety. That is not true for the workflow Orbit actually uses: agents commonly work in separate worktrees and branches, each with its own graph ref. A lock inside a branch-local graph snapshot cannot reliably serialize writes in another worktree.

## Decision
Remove graph mutation tools from the public tool/MCP surface. Keep the current agent-facing graph API read-only: overview, search, show, pack, refs, callers, implementors, and deps. Use `task_gate_pipeline` and its `reserve_locks` activity to reserve task `context_files` as this version's preflight write guard, with optimistic integration/review checks as the final authority for stale or overlapping edits. Internal working-graph and operation-log code may remain as deferred implementation substrate, but it is not advertised as a current agent API.

## Consequences
- Agents no longer see graph writes as a supported coordination mechanism.
- Write admission happens in a shared task/workflow plane rather than inside per-ref graph state, so it still has meaning before agents fan out into separate worktrees.
- Graph refs remain a read/index/context artifact, which matches their branch-scoped storage model.
- Cost: write guards are conservative at task context-file granularity, not precise at graph-node granularity. Fine-grained symbol-level mutation may return later only with a coordination story that works across worktrees.

---
