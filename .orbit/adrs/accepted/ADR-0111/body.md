## Context
A learning's scope (when does it match?) and ranking (which match wins?) have multiple plausible designs:

| Scope axis | Profile |
|------------|---------|
| **Path globs** | Match against file paths the agent is about to touch. Stable shape, simple matcher (reuses `orbit-policy`'s glob engine). Brittle to file renames. |
| **Tags** | Free-form labels. Survive renames. Require the author to anticipate the categorization. |
| **Symbol IDs** | Match against knowledge-graph symbols. Survive renames cleanly. Couples to graph rebuilds. |
| **Semantic similarity** | Match by embedding distance to current edit context. Catches relevance the other axes miss. Depends on semantic-search infrastructure. |

| Ranking | Profile |
|---------|---------|
| **Recency (`updated_at` desc)** | Trivial. Wrong when an old, important learning loses to a recent, marginal one. |
| **Manual `priority`** | Author-supplied. Honest signal when used; degenerates to "everything is high priority" without curation discipline. |
| **Semantic similarity** | Best signal. Requires embeddings. Cost = embed every learning + run cosine on every query. |

Phase 1's binding constraint is: ship before semantic-search reaches Accepted ([T20260510-3]). That rules out semantic similarity for both scope and ranking. Symbol-aware scope is *technically* available — the knowledge graph already exists — but coupling the learning store to graph rebuilds adds dependency surface and mainly pays off when fused with semantic ranking. Doing one without the other yields a clunky middle state.

## Decision
Phase 1 supports two scope axes, evaluated as logical OR: path globs (matched via the `orbit-policy` glob engine) and tags (matched as exact strings). Ranking is `updated_at` desc with optional `priority` tagging as a tie-breaker. The schema reserves `scope.symbols` and `scope.semantic_seed` fields for phase 2 forward compatibility, but neither is read in phase 1.

Phase 2 ([3_vision.md §1.1](./3_vision.md), [§1.2](./3_vision.md)) layers symbol-aware scope and semantic ranking once semantic-search ships.

## Consequences
- Phase 1 is implementable in parallel with semantic-search work, not gated on it.
- Path globs cover the common case (most learnings are file-area-scoped) and tags cover the cross-cutting case.
- The schema is forward-compatible; phase 2 is additive, not a migration.
- Cost: recency-only ranking has known failure modes ([3_vision.md §1.2](./3_vision.md)) — old-but-important learnings get out-ranked by recent-but-marginal ones. Path globs are brittle to renames; the documented mitigation is "run `orbit learning prune --stale-only` after refactors that move files," which is operational discipline, not automation. Both costs are accepted as the price of shipping phase 1 ahead of semantic-search.

---
