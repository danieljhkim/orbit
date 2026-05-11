## Context
Nodes already carry `task_ids` from the attribution pass, but agents had no inverse lookup for "which selectors did this task touch?" A dedicated sidecar index could make that lookup faster, but it would add another persisted source of truth before usage patterns justify it.

## Decision
Add `task_id` as an optional filter on `GraphContextService` search and expose it through `orbit.graph.search`. The filter exact-matches one task ID against each node's existing `task_ids` vector and composes with the existing query/type/kind/prefix/source-regex filters.

## Consequences
- Agents can answer review-prep and incident-inspection questions from the existing graph snapshot.
- No new graph schema, sidecar, or invalidation path is introduced.
- Cost: lookup remains O(nodes) and multi-task queries require repeated calls until a real usage pattern justifies an indexed or array-based surface.

---
