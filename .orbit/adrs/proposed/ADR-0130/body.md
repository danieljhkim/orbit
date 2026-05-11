## Context
The biography surface keys every story on a KG `stable_id`. If `stable_id` does not survive the structural changes lineage is meant to outlive (renames, file moves, refactors), biographies orphan and the surface degrades silently. Per [knowledge-graph ADR-010](../knowledge-graph/4_decisions.md), `stable_id` is intended to survive renames and file moves; per [2_design.md §7.2](./2_design.md), it is known *not* to survive extract-into-multiple, inline-into-caller, cross-language ports, and wholesale rewrites. Two responses were on the table. Response A: invest in KG identity-tracking heuristics (similarity matching, AST-shape stitching) to close the orphan cases before lineage ships. Response B: ship lineage on top of today's KG identity, name the orphan cases in the renderer's output, and treat orphan-rate reduction as a follow-up KG investment.

## Decision
Response B. Lineage's biography surface ships on top of today's KG `stable_id` semantics. The renderer surfaces the orphan failure mode explicitly via a footnote when a symbol's biography is shorter than its containing-file biography ([2_design.md §7.4](./2_design.md), §6.2). Mitigations on the lineage side (cross-rename evidence accumulation, path-grain fallback, manual `same_as` stitch) cover the most common cases. Investment in KG identity heuristics is a follow-up gated on the measured orphan rate ([2_design.md §9.4](./2_design.md)).

## Consequences
- Lineage's correctness ceiling is bounded by KG's identity-tracking ceiling. This is honest and visible to the agent, not hidden behind the renderer.
- The orphan rate becomes a measurable input that drives future KG investment. If biographies orphan often in practice, the response is to improve KG identity, not to weaken the biography surface.
- Phase 1 ships a feature that is sometimes silently incomplete (when the renderer's footnote heuristic misses an orphan case). This is acceptable for a development-time tool but would not be acceptable for a release-time integrity surface.
- Cost: every Phase 1 user encounters orphan biographies in some cases and has to evaluate the footnote. The biography surface is *not* a guarantee; it is a best-effort with provenance. If the orphan rate proves higher than the [§9.4](./2_design.md) 10% threshold, Phase 1's value proposition takes a hit and the kill criterion in [ADR-001](#adr-001--restore-code-graph-task-attribution-as-the-lineage-consumer) becomes more likely to fire.

---

## Task References

- [T20260506-11] — Removed graph task attribution; supersession candidate ([ADR-001](#adr-001--restore-code-graph-task-attribution-as-the-lineage-consumer)).
- [T20260506-15] — Added `orbit.task.locks.reserve` `files` shape; lineage exemplar.
- [T20260510-17] — Removed agent-facing lock instructions; authoring failure that surfaced lineage as a need.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
