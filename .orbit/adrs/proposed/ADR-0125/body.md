## Context
Orbit task IDs are workspace-local per [task-sync 2_design.md](../task-sync/2_design.md) and per [knowledge-graph ADR-029](../knowledge-graph/4_decisions.md)'s framing. A "global" lineage graph that merges two engineers' machines requires distributed-consistency machinery far beyond lineage's scope. But cross-engineer presentation is a real need — PR descriptions, audit reports, and shared root-cause analyses want to surface lineage to readers on other machines.

## Decision
Lineage chains stay strictly local. The workspace-local commitment is load-bearing for the minimal Phase 1 (it is why the substrate avoids distributed-state machinery entirely). The render-time `external_refs` projection that surfaces cross-engineer output (PR descriptions, audit reports) is **deferred from the minimal first draft** — it is a presentation concern, not a substrate concern, and its specification waits until a consumer that emits cross-engineer output ships.

## Consequences
- The local graph stays dense, fast, and consistent. No distributed-state CRDT, no cross-machine sync, no eventual-consistency reasoning. (This part is in scope for the minimal Phase 1.)
- Cross-engineer presentation will eventually use external IDs as the cross-machine spine ("[ENG-1234] (T20260510-17 on author-machine)"). The render shape is named here as a placeholder; the actual implementation lands with the first consumer that needs it.
- A workflow that genuinely needs to merge two machines' lineage (e.g., shared root-cause analysis) requires an explicit publish/import step, not a live distributed graph. [3_vision.md §1.9](./3_vision.md) names the open question on what the publish surface looks like.
- Cost: cross-engineer reasoning loses transitive closure. If engineer A's machine has the chain A1→A2 and engineer B's machine has B1→B2, neither machine sees the merged A1→A2↔B1→B2 chain even when both pairs share an external_ref. Acceptable for development-time tools; not acceptable if lineage ever becomes a release-time integrity surface (which would require reconsidering this ADR).

---
