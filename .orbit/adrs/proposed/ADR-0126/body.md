## Context
Two API shapes are common for graph systems. Shape A: low-level traversal primitives (vertex, edge, neighbor) with consumers composing their own queries. Shape B: named, opinionated closures (`lineage`, `feature`, `decision`, `reversal`, `risk`) with raw enumeration as an escape hatch. Shape A is more flexible; Shape B is more legible.

## Decision
Shape B. The named closures in [2_design.md §5](./2_design.md) are the primary agent-facing surface — minimal Phase 1 ships `feature` and `biography`; additional closures (`closure`, `decision`, `reversal`, `risk`) are deferred and ship with their consumers. `orbit.lineage.edges` exists as a raw escape hatch but is not advertised in the standard skill or in the primary tool reference. Closures default to opinionated parameters (edge-type sets, depth limits, confidence thresholds) chosen to fit each use case.

## Consequences
- Agents reason about *named closures* with stable contracts, not about graph mechanics. The skill instructions for `orbit-create-task` can say "compute lineage closure" without specifying which edge types or depth — the closure tool knows.
- Adding a new use case is adding a new named closure, which is more visible and reviewable than letting consumers compose ad-hoc traversals.
- Closure parameter defaults become load-bearing UX decisions; bad defaults silently degrade every consumer.
- Cost: any consumer with a use case the five closures don't cover has to fall back to `edges` and write traversal logic. A consumer that *should* be a sixth named closure but ships as a raw-edges consumer is a soft drift that's hard to detect. Mitigated by treating the named-closure list as a public-API growth list, with new closures requiring an ADR addition (this one).

---
