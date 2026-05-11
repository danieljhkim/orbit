## Context
Orbit carried a task attribution pipeline that parsed task IDs from commit messages, mapped hunks back to graph nodes, persisted `task_ids`/`structural_conflict` fields, wrote a task-commits sidecar, and exposed reverse lookup through `orbit.graph.search task_id` and `orbit.graph.history`. A 10-day audit window from 2026-04-26 to 2026-05-06 found 961 `orbit.graph.*` tool calls and 0 uses of the reverse-lookup parameters. The forward lookup users actually need is already native git text search: `git log --grep '[T<task-id>]'`. Separately, the task-sync doctrine now treats Orbit `task_id` as local-only; cross-engineer references go through `external_refs`.

## Decision
Remove graph task attribution. Delete the attribution pipeline, `TaskIdPattern`, task-id search/history parameters, node attribution fields, sidecar persistence, and manifest/config plumbing. Keep `orbit.graph.history` as a compatibility stub that returns a clear removal message and points to `git log --grep`. Preserve commit-message `[T...]` convention as a local search key.

## Consequences
- Graph build no longer pays attribution-walker cost or persists unread reverse-lookup data.
- Legacy graph objects and refs that still contain attribution fields load through serde unknown-field tolerance; the fields disappear on rebuild.
- `knowledge.task_id_pattern` is deprecated and ignored with a one-line warning instead of failing old configs.
- Cross-engineer task references are explicit through `external_refs`, not inferred from local Orbit task IDs.
- Cost: users who depended on reverse lookup from selector to tasks lose that graph query. The documented replacement only covers forward lookup from task ID to commits.

---
