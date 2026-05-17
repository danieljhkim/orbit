## Context
Orbit exposed `ship`, `ship-local`, and `ship-auto` as separate shipment aliases. Operators had converged on `ship-auto` because it already parks work behind dependency and lock gates, while explicit `ship <TASK_ID>` still failed fast before the waiting-reason surface from ORB-00074 could explain parked work.

## Decision
Expose one public command, `orbit run ship`, where omitted task ids run backlog auto-selection and provided task ids seed the same gated path. The command submits `task_auto_pipeline` for both forms, preserves `mode` and `base_branch` inputs, returns after `submit_pipeline_run`, and keeps `ship-auto` only as a deprecated CLI form that errors toward `orbit run ship`; `ship-local` is no longer a workflow alias.

## Consequences
- Explicit task selection now queue-and-waits inside `task_gate_pipeline` instead of failing fast at CLI dispatch.
- Operators inspect waiting, queued, and terminal state through `orbit run history -j task_auto_pipeline` and `orbit run show <RUN_ID>`.
- The job-routing rule stays simple: `task_auto_pipeline` lists explicit ids as singleton bundles and each bundle fans into the existing gate before `task_{{ input.mode }}_pipeline`.
- Cost: dispatch output no longer includes the former synchronous ship-auto status summary because terminal pipeline state is not available at submit time.