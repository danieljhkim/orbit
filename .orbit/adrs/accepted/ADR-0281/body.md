## Context

Orbit had three shipment aliases: `ship`, `ship-local`, and `ship-auto`. Operators used the auto path because it already queued behind dependency and lock gates, while explicit shipment still failed fast before the waiting-reason surfaces could explain parked work.

## Decision

Use `orbit run ship` as the only public shipment command. Omitted task IDs run backlog auto mode, provided task IDs seed explicit singleton bundles, and both forms submit `task_auto_pipeline`; mode still routes inside `task_gate_pipeline` to `task_{{ input.mode }}_pipeline`. The companion global ADR is ADR-0152.

## Consequences


- Explicit task selection now waits inside the gated job path instead of failing at CLI dispatch time.
- `orbit run ship` returns after `submit_pipeline_run`, and operators inspect waiting or terminal state with `orbit run history -j task_auto_pipeline` and `orbit run show <RUN_ID>`.
- The deprecated `ship-auto` CLI form errors toward `orbit run ship`, and `ship-local` is no longer a workflow alias.
- Cost: dispatch output no longer contains the former synchronous auto-shipment summary because terminal pipeline state is unavailable at submit time.

## Provenance

Migrated verbatim from the local heading `activity-job/ADR-052` in `docs/design/activity-job/4_decisions.md` by [ORB-10458]. Original status line: Proposed · 2026-05 · [ORB-00075]