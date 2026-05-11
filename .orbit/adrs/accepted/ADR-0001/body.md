## Context
Activity/job correctness depends on making authoring conveniences disappear before execution. The old log carried separate ADRs for schema retirement, backend resolution, target refs, defaults, catalog precedence, seeded assets, and workflow admission, but all enforce the same boundary: YAML is human-authored input, while execution sees normalized, validated runtime state.

## Decision
Treat `schemaVersion: 2` as the only activity/job asset family, load seeded and workspace catalogs with explicit layer precedence, resolve authoring sugar (`target: activity:<name>`, `backend: auto`, object-valued defaults, and workflow admission) before dispatch, and keep seeded activities/jobs as executable reference contracts for that normalized surface. Direct task-workflow admission remains a workflow-specific normalization path rather than a generic task-update rule.

Folded instances:

| ADR | Instance folded into this rollup |
|-----|----------------------------------|
| ADR-003 | `backend: auto` resolves once before dispatch. |
| ADR-004 | `target: activity:<name>` is authoring sugar resolved before execution. |
| ADR-008 | Seeded activities and jobs are load-bearing runtime contracts. |
| ADR-011 | Object-valued job defaults shallow-merge with caller input, and early failures get synthetic job steps. |
| ADR-013 | Job catalog discovery honors layer precedence. |
| ADR-016 | Activity catalog discovery honors layer precedence, and activity execution stays job-owned. |
| ADR-026 | Workflow admission is distinct from generic task updates. |

## Consequences
- The runtime now documents and validates one typed activity/job surface.
- Human-authored YAML stays readable while executors consume concrete steps, concrete backends, merged inputs, and first-wins catalog entries.
- New workspaces start with real executable reference assets rather than empty examples.
- Costs retained from folded entries:
- Cost: old assets stop limping along; migration work becomes mandatory instead of gradual.
- Cost: callers must remember to run the normalization pass before dispatch, and any missed call site fails as a structural bug.
- Cost: the load path owns more normalization logic, and stale refs fail before dispatch instead of being lazily recoverable.
- Cost: seeded assets become part of the public maintenance burden and can drift if docs/tests stop exercising them.
- Cost: the job-level input contract is now a shallow merge rule that docs and tests must preserve, and run history can include synthetic job-level failure steps that were not literal authored YAML steps.
- Cost: lower-precedence job assets can be shadowed silently, so debugging an unexpected workflow now requires checking catalog source paths.
- Cost: lower-precedence activity assets can be shadowed silently, and direct ad hoc activity execution is no longer a documented CLI workflow.
- Cost: task lifecycle semantics are no longer uniform across all status mutation surfaces; reviewers must distinguish workflow admission from ordinary task updates.
