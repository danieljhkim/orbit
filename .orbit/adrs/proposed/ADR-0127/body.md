## Context
Two architectural shapes were on the table. Shape A: a standalone task lineage graph that *references* code-graph nodes via stable IDs in evidence fields, with the two graphs traversed separately by consumers. Shape B: an explicit bipartite graph with task↔task edges and task↔code edges as first-class peers, queryable as one substrate.

## Decision
Shape B. The task graph and the code knowledge graph are one bipartite graph. `task_code_edges` is a peer table to `task_edges` ([2_design.md §1](./2_design.md)). Closure operations cross the two planes freely. The KG remains the authoritative source for code-node identity (stable_id, structure); the lineage system owns only the attribution edges.

## Consequences
- Cross-plane queries are first-class. "What code did the lineage of this symbol's tasks touch?" is one CTE, not two queries with manual join.
- The lineage system's correctness is partly the KG's correctness ([2_design.md §8](./2_design.md) names this honestly). A KG attribution bug becomes a wrong lineage edge.
- The bipartite shape is what makes lineage's ambition load-bearing rather than aspirational ([3_vision.md §3.1](./3_vision.md)). Without it, lineage is just typed Jira links.
- Cost: lineage's evolution couples to KG evolution. Schema changes on either side require coordination. Workspace-local store contains two large derived indices instead of one — disk and rebuild cost both grow.

---
