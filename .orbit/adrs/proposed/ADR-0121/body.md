## Context
[Knowledge-graph ADR-029](../knowledge-graph/4_decisions.md) removed graph task attribution citing a 10-day audit with 961 `orbit.graph.*` calls and 0 reverse-lookup uses. The audit was correct in its window — there was no consumer of reverse lookup. Lineage is the consumer the audit didn't have: the symbol-grain `co-touches-symbol` edges, the `feature` closure ([2_design.md §5.1](./2_design.md)), and the symbol-biography renderer ([2_design.md §6](./2_design.md)) all require per-node `task_ids` attribution.

## Decision
Restore symbol-grain task attribution on knowledge-graph nodes, narrowed to what lineage's derivation pipeline consumes: `task_ids: Vec<String>` (set, not ordered) on each node, populated by a single pass over commits in the build window. Drop the historical hunk-mapping walker, the `structural_conflict` field, and the task-commits sidecar — those carried cost (per ADR-029, the dominant cost was hunk-coordinate remapping after rename hops) without serving lineage's consumers. Flip ADR-029 to `Superseded by [this ADR + implementing task]` when the implementation lands.

## Consequences
- Lineage's `kg-attribution` deriver becomes a thin reader of node `task_ids` rather than reinventing attribution.
- `orbit.graph.history` graduates from compatibility stub back to a real graph tool, with shape narrowed to what lineage needs (forward and reverse, file and symbol, no per-hunk).
- The audit's "0 reverse-lookup uses" finding stays valid for the *old* shape; the new shape has exactly one well-defined consumer at rollout, with kill-criterion below.
- **Kill criterion.** If after 90 days post-rollout the lineage authoring-assist hook fires fewer than 5 times per active workspace per week (median), revert this ADR rather than invent more consumers to justify the cost. The rollback is mechanical: drop the `task_ids` field from node serialization, restore the compatibility stub, mark this ADR `Superseded` by the rollback task.
- Cost: build-time pays for the attribution pass on every graph rebuild — bounded but non-zero. Migration cost: existing graph refs without attribution rebuild once, then carry the new field. Schema-shape risk: if a future deriver wants per-hunk attribution, ADR-029's fuller machinery has to be re-introduced (avoidable now, expensive later if needed).

---
