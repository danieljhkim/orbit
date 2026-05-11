## Context
SQLite command-audit rows recorded tool invocations but had no direct link to the task, job run, activity, or step that caused them.

## Decision
Add nullable `task_id`, `job_run_id`, `activity_id`, and `step_index` columns, populate them at runtime tool dispatch from caller JSON first and engine env vars second, index task/run ids, and render the fields in dashboard detail rows.

## Consequences
- Operators can drill from a tool row to the originating task and run context without out-of-band correlation.
- Cost: historical rows remain NULL, and caller-asserted JSON values are weaker evidence than engine-supplied env context.
