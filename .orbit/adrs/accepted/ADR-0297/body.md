## Context

A ship-pipeline (`workflow_ship`) implementation agent is a CLI subprocess spawned by the `agent_implement` activity inside the pipeline worker. Bridge `agent_run_list` only observes the separate Worker daemon behind `agent_invoke`, so these children have no run-store row and no exposed identity: `child.id()` was used only inside `cli_runner/supervisor.rs` for process-group cleanup. During run-rescue (F2026-07-083, ORB-10257) a healthy long-running Sol/Codex agent was therefore indistinguishable from a dead child — provable only by shell process-tree inspection — which risks an operator cancelling legitimate in-flight work.

Two shapes were available for making the child observable while it runs:

(a) A heartbeat: have the supervisor periodically write a liveness timestamp (audit row or run-record column) for as long as the child is alive, and let readers compare it against now.

(b) A single spawn-time record of the PID plus its process-start identity token, with liveness probed by the reader at query time.

Extending the existing `cli.invocation.started` event was not an option: it is emitted before spawn, by construction, so no PID exists yet.

## Decision

Emit one new `cli.invocation.process` v2 audit event (`provider`, `pid`, `pid_start_time`) immediately after spawn and before the supervision loop, ordered strictly between `cli.invocation.started` and `cli.invocation.finished`. The envelope writer persists synchronously, so the row is readable while the invocation is still running.

Liveness is computed at read time, not stored. `orbit_common::utility::process_identity::probe_process_liveness` answers `alive` / `exited` / `unknown` from `kill(pid, 0)` plus the Linux zombie check, using the recorded `pid_start_time` token to reject a recycled PID. `OrbitRuntime::collect_run_provider_processes` pairs each process event with the `cli.invocation.finished` event that closes it within the same step and probes only the still-open ones. `GET /api/runs/:id` (bridge `workflow_run_status`) and `orbit run show` project the result.

## Consequences

- A long-running `agent_implement` step is distinguishable from a lost child without shell access to the host, which is the operator decision run-rescue actually has to make.
- Retries within one step pair up in order (newest still-open record wins), so a step that respawned its provider reports each attempt separately rather than collapsing onto the first spawn.
- PID reuse cannot fake a live agent: a live PID whose versioned start-identity token disagrees reads as `exited`.
- An unprobeable host degrades to `alive`/`unknown` rather than `exited` — a probe that cannot answer must never be read as proof of death, matching the existing job-run owner-reconciliation policy.
- Cost: liveness is only as fresh as the moment it is queried and only meaningful on the host that ran the child. A remote or later reader of the same audit trail gets `exited` for every historical open invocation, because the answer is derived from the local process table rather than persisted with the event. A heartbeat would have survived that, at the price of a write per interval per invocation and a staleness threshold to tune.
- Cost: `pid_start_time` costs one `ps` invocation per provider spawn. A sandbox that blocks `ps` yields `None`, which weakens the record to unguarded-PID liveness rather than failing the spawn.