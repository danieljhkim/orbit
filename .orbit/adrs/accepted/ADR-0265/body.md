## Context

Task links currently appear as `parent_id`, `dependencies`, `source_task_id`, and external references, while execution fan-out currently uses `batch_id` for job-run membership. The first three are task-to-task relationships; `batch_id` is really a foreign reference to an execution/job run and should be named `job_run_id`.

## Decision

Use a directed `relations` array with explicit relation types for task-to-task links, and store `job_run_id` as a separate optional envelope attribute. The v2 envelope stores one typed relation surface and does not retain `parent_id`, `dependencies`, `source_task_id`, or `batch_id` as compatibility fields.

Relation types are source-implied: `child_of`, `blocked_by`, `spawned_from`, `regression_from`, `supersedes`, and `related_to`. This means a task that depends on another task carries `blocked_by -> dependency`, and a subtask carries `child_of -> parent`. Writers reject self-edges, duplicates, and cycles for hierarchy/blocking relation families; generated indexes materialize relation and inverse lookup rows.

## Consequences


- Consumers can traverse relationships by meaning instead of hardcoded field names.
- Future relation types can be added without widening the top-level envelope.
- Task lineage can share vocabulary with the task artifact rather than deriving every edge from prose.
- Common create flows write only the new task bundle; they do not need a fan-out update to the parent or dependency task.
- Job-run filtering is explicit and indexed through `job_run_id`, not smuggled into task relations.
- Cost: relation validation becomes stricter and more complex. Existing callers that set old link fields must be updated to write the typed relation surface.

## Provenance

Migrated verbatim from the local heading `task-artifacts/ADR-005` in `docs/design/task-artifacts/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-05 · Phase 6 relations and job-run wiring (working tree)