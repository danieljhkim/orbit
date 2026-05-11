## Context
Retrying from a dirty workspace is the main failure mode Groundhog is trying to avoid. The rewind mechanism also needs to survive crashes and remain inspectable after a failed attempt.

## Decision
Each attempt executes on a scratch branch named `groundhog/<task_id>/day-<n>` and rewinds by resetting the task branch back to `snapshot_ref`.

## Consequences
- Failed attempts leave behind inspectable scratch branches.
- Success can be materialized as one squash commit per checkpoint.
- Cost: scratch branches proliferate during long runs and need cleanup discipline.
