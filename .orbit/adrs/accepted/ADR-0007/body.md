## Context
The `epic_orchestrator` activity exists to make one judgment cycle: read the deterministic epic snapshot, choose ready bundles, and dispatch child `task_gate_pipeline` runs. Its previous instruction also made the HTTP agent call `orbit.pipeline.wait`, but a normal gate-and-ship envelope can exceed the orchestrator's wall-clock by hours: gate admission can wait, child dispatch can wait, and implementer activities have their own long timeout.

## Decision
Keep the orchestrator fire-and-forget. It may call `orbit.pipeline.invoke`, then must return structured `dispatched_run_ids`. `task_epic_pipeline` performs the blocking join through deterministic `pipeline_wait`, then runs `refresh_epic` so loop exit still keys off durable task state. The per-cycle wait budget should satisfy `iteration_wait_seconds >= task_gate_pipeline.max_wait_seconds + task_gate_pipeline.dispatch_timeout_seconds` for full-envelope joins; seeded defaults currently keep `iteration_wait_seconds` at the pipeline wait cap of 7200 seconds, below the theoretical 10800-second gate envelope, so a timeout can surface a still-running child.

## Consequences
- A premium HTTP orchestrator session is bounded to a dispatch decision cycle instead of babysitting child workflow polling.
- Audit lineage moves from agent tool calls to deterministic `ActivityStarted` / `ActivityFinished` envelopes for the join step; the child relationship remains reconstructable from `dispatched_run_ids` and run-step state.
- If `pipeline_wait` times out while a child is still running, the next deterministic `load_epic` snapshot still shows open work. Redundant redispatch is bounded by the gate pipeline's task-lock reservation: overlapping context files are denied while the child reservation is active, and TTL remains the abandoned-run fallback.
