## Context
The simplest schema for inter-task linkage is a single `related_task_ids: Vec<String>` field on the task record. That shape is universal in issue trackers (Jira "links" panel, GitHub "linked issues"). It also has a universal failure mode: an undifferentiated relation set means consumers can't reason about *kinds* of relationships (supersession vs co-touch vs runtime-parent), so closure operations either treat all edges identically (loss of signal) or rely on text-parsing each edge's surface (fragile).

## Decision
Edges are typed. The Phase 1 edge type set in [2_design.md §1.1](./2_design.md) (7 task-task plus 4 task-code) forms the system's causal vocabulary; additional types named in that section are deferred. Each edge carries `(from, to, type, source, confidence, evidence, derived_at)`. Closure operations parameterize on edge-type sets so different consumers can dial in a different filter.

## Consequences
- Consumers can reason about edge *meaning*, not just edge *presence*. "Tasks that supersede this one" is a different query from "tasks that co-touch this one."
- Adding a new type after rollout requires every consumer to handle it gracefully or be updated. We chose 12 types up front (a deliberate over-shoot) to minimize post-rollout schema churn — see [3_vision.md §1.3](./3_vision.md) for the open question on evolution.
- The taxonomy itself becomes load-bearing documentation: agents authoring tasks learn the edge vocabulary as part of authoring.
- Cost: any scheme where edges must be classified at derivation time is more code than a flat link list. The `task-text` deriver's verb upgrade rules ([2_design.md §2.2](./2_design.md)) are an explicit complexity tax we pay for typed-edge fidelity. Edge-type taxonomy churn is more painful than untyped-link-list evolution.

---
