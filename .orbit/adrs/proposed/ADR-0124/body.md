## Context
Two storage shapes were on the table. Shape A: edges as fields on the existing per-task YAML (e.g., `edges: { supersedes: [T...], extends: [T...], ... }`). Shape B: a SQLite table separate from the task record. Shape A is consistent with the existing layered store pattern (YAML + SQLite) and keeps task metadata co-located. Shape B requires a new table and a new write path.

## Decision
Shape B. Edges live in `task_edges` and `task_code_edges` SQLite tables ([2_design.md §1](./2_design.md)). The per-task YAML stores no edge fields beyond the existing `dependencies` (reused for declared blocker edges).

## Consequences
- Closure queries are recursive CTEs on a real index, with millisecond latency at expected workspace scales.
- Task YAML stays small and human-readable. Derived edges, which can run into the thousands per workspace, never bloat task files.
- Cursor state, derivation timestamps, and provenance evidence live alongside the edges and don't pollute task records.
- Reading edges does not require reading task YAML; reading task YAML does not require reading edges. The two surfaces are independently consumable.
- Cost: the storage layout carries one more table and one more write path. Backup, sync, and migration logic must cover both. Reviewers who want to see all of a task's relationships must consult two surfaces, not one — mitigated by `orbit.task.show` rendering edges inline as a derived field in JSON output (write-only on the YAML, read-only on the rendering).

---
