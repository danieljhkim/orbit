## Context
The dashboard Tasks tab was read-only for the operator two most common actions: dispatching a backlog task and leaving a note on one. Both are writes against live state, so the question was how much configuration to expose and whose identity to record.

## Decision
Ship is one click with no configuration UI: the dashboard posts only the task id. The crew comes from the task record and the mode from the workspace registry binding, so the ship endpoint omitted-mode default changes from a hard-coded pr to that binding ship mode. Duplicate dispatch is refused server-side with 409 ship_run_in_flight when the task already has a non-terminal run. Comments post to POST /api/tasks/:id/comments, writing into the task existing review-thread structure and forcing a human author.

## Consequences
- Triage, dispatch and annotation complete inside the dashboard.
- A dashboard comment is always attributable to a person, even when the server runs inside a managed Orbit run.
- Cost: the ship endpoint scans a bounded window of recent runs before submitting, and a stuck non-terminal run must be cancelled before its task can be re-shipped.