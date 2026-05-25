## Context
The graph query surface was still split: MCP already used orbit-graph, while the CLI tool registry still routed graph queries through orbit-knowledge. ORB-00338 extended the fixture-scoped equivalence corpus to 34 selectors across sync, search, show, refs, callees, impact, and trace, then measured both backends on the same query set.

## Decision
Cut over the in-process/CLI graph query tools to orbit-graph only: sync, search, show, refs, callees, impact, and trace. Do not add a runtime backend selector, environment toggle, audit-row backend column, or shadow-diff logger. Rollback is the normal Git operation `git revert <cutover-sha>`; because this executor does not create the final commit, conflict-free rollback verification is pending the commit/PR step with the exact cutover SHA.

## Consequences
- Corpus scope: fixture-scoped but reviewed from real Orbit graph-query shapes, including command trace and explicit sync, while keeping v1 extraction fixture-scoped to avoid the known full orbit-knowledge refresh cost.
- Equivalence result: 34/34 passed; zero diff rows, so there were zero `bug-in-new`, zero `improvement-over-legacy`, and zero uncategorized diffs.
- Perf result from the attached ORB-00338 graph-equiv report: v1 median 8us / p95 16us; v2 median 6870us / p95 7780us. v2 includes subprocess query overhead in this harness and remains acceptable for the cutover scale.
- No single code anchor; this is a cross-surface migration decision enforced by the graph tool registry and CI equivalence harness.
- Cost: The CLI/agent graph surface drops legacy orbit-knowledge-only helpers (`pack`, `overview`, `callers`, `implementors`, `deps`) in favor of the smaller seven-tool orbit-graph surface. Source-regex enumeration and multi-selector pack workflows now fall back to regular file reads or narrower graph queries when needed.