## Context
The retained `run_parallel_task_pipeline` automation path used scoped threads to call `run_job_now_with_input_debug`, then marked active workers failed after a long receive timeout. Rust scoped threads still join before the scope exits, so a never-returning worker could keep the parent dispatcher hung even after timeout failure recording.

## Decision
Launch each legacy parallel-batch worker through the durable pipeline surface (`orbit.pipeline.invoke`) and poll active run IDs through `orbit.pipeline.wait` instead of owning scoped worker threads. When the configured worker timeout elapses, the dispatcher cancels every active child run before writing `WORKER_TIMEOUT` task failure state and returning the batch failure.

## Consequences
- Timeout return no longer depends on the worker's thread or agent process eventually exiting.
- Timed-out child work gets the same run-cancellation path operators use elsewhere, including bounded process-group signaling for running pipeline workers.
- Cost: the retained legacy path now depends on the v2 pipeline tool surface and polls active workers, so completion can lag by the polling interval rather than waking on an in-process channel send.

---

## Task References

- **[T20260418-2018]** — Add `JobV2` DAG constructs (`parallel`, `fan_out`, `loop`, `retry`, `when`).
- **[T20260418-2019]** — Add v2 activity name resolution and pipeline skeleton assets.
- **[T20260418-2143]** — Wire `V2RuntimeHost` in orbit-core and add `orbit activity run-v2`.
- **[T20260418-2210]** — Reshape `V2RuntimeHost` to keep `orbit-agent` types out of orbit-core.
- **[T20260419-0002]** — Add `workspace_path` provenance to the v2 audit envelope.
- **[T20260419-0104]** — Add `backend: cli` dispatch for v2 `agent_loop`.
- **[T20260419-0622-3]** — Add `task_gate_pipeline`.
- **[T20260419-0623]** — Add `task_auto_pipeline`.
- **[T20260419-0623-2]** — Add `task_epic_pipeline`.
- **[T20260419-2014]** — Merge `orbit-types` into `orbit-common`.
- **[T20260419-2156]** — Retire v1 assets and drop the transitional v2 naming.
- **[T20260419-2347]** — Seed activities and workflows on `orbit init`.
- **[T20260420-0510-2]** — Add the Groundhog v1 activity runner.
- **[T20260421-0542-2]** — Add structured `list_backlog_tasks` output for context-lock exclusions.
- **[T20260423-0114]** — Expose the `backend: cli` executor-args gap during a local task ship run.
- **[T20260423-0445]** — Merge object-valued job defaults over explicit run input and persist synthetic failed job steps for early v2 pipeline failures.
- **[T20260423-0447]** — Restore usable `orbit run duel` read-only surfaces after duel workflow retirement.
- **[T20260423-2004-4]** — Persist direct v2 `orbit job run` executions into durable job-run records and state.
- **[T20260425-0204]** — Make v2 job catalog discovery honor workspace-over-global `MergeByKey` precedence.
- **[T20260425-2010]** — Refactor `orbit run` task workflow commands and revive `duel-plan` as a seeded run workflow.
- **[T20260426-0047]** — Make v2 activity catalog discovery honor workspace-over-global `MergeByKey` precedence and remove the public `orbit activity run` command.
- **[T20260426-0526]** — Restore v2 job invocation trace persistence so `orbit metrics` can report agent and tool usage.
- **[T20260426-0519]** — Move file-backed activity/job audit traces under `.orbit/state/audit`.
- **[T20260426-0705]** — Expose v2 run audit events through `orbit run events` and `orbit run trace`.
- **[T20260426-0709]** — Align run step selectors on activity `step.id` and move CLI invocation log reading behind orbit-core runtime accessors.
- **[T20260426-0742]** — Remove duplicate job-level run inspection aliases and keep run inspection under `orbit run`.
- **[T20260426-2313]** — Stream CLI subprocess stdout/stderr through structured tracing events while retaining the existing audit/blob path.
- **[T20260426-2349]** — Move CLI tracing output redaction from `cli_runner` call sites into the default tracing formatter layer.
- **[T20260427-33]** — Remove the audit-only `dispatch_agent` step from `task_auto_pipeline`.
- **[T20260427-34]** — Add seeded pipeline success guards so non-succeeded child runs fail parent shipment workflows.
- **[T20260427-36]** — Align task-gate reservation TTL with the child dispatch wait budget.
- **[T20260427-38]** — Treat review as a shipped stop state for epic automation.
- **[T20260427-40]** — Move epic child-run waiting out of the orchestrator agent and into a deterministic workflow step.
- **[T20260427-45]** — Use freshly fetched remote base refs for default task-shipping worktrees.
- **[T20260427-48]** — Thread provider config into the v2 CLI backend and keep Codex dynamic flags exec-compatible.
- **[T20260428-8]** — Add workflow-specific task admission for task-starting workflows.
- **[T20260428-9]** — `orbit init` writes per-role agent settings to `[agent.<role>]` in `config.toml`.
- **[T20260428-12]** — Wire `[agent.<role>]` config into `agent_loop` dispatch via the `role:` field and a host-backed resolver.
- **[T20260430-9]** — Add a job-level recovery activity hook for retry-exhausted v2 step failures.
- **[T20260430-12]** — Ship a generic deterministic recovery activity for direct task shipment workflows.
- **[T20260430-14]** — Make default step recovery agent-driven and step-scoped.
- **[T20260509-14]** — Reuse the configured reviewer role for step-failure recovery.
- **[T20260430-15]** — Embed task-aware input and run context in backend: cli agent envelopes.
- **[T20260430-19]** — Shorten the Activity / Job design docs while preserving required structure.
- **[T20260430-26]** — Release task-gate reservations after terminal child shipment runs and expose active reservations through the lock view.
- **[T20260430-27]** — Make `ship-auto` output distinguish empty backlog, gated no-op, and waiting gate children.
- **[T20260430-30]** — Make `ship-auto` default text output human-readable while preserving JSON fields.
- **[T20260430-31]** — Require populated execution summaries before opening task PRs.
- **[T20260505-2]** — Admit accepted backlog friction reports in automatic backlog listing.
- **[T20260505-8]** — Add dashboard/runtime controls to cancel active job runs.
- **[T20260505-10]** — Release run-owned task lock reservations through engine-owned terminal cleanup and reserve-pressure reconciliation.
- **[T20260505-22]** — Rewrite Claude's `--debug-file` static arg at dispatch time so the log lands at a sandbox-allowed absolute path.
- **[T20260506-16]** — Replace raw `orbit init` agent prompts with a recommendation-first setup wizard.
- **[T20260506-17]** — Make `orbit init` recommend Codex for reviewer and implementer when available.
- **[T20260506-18]** — Compact activity-job ADRs via rollups.
- **[T20260508-3]** — Revise generated task PR bodies around the one-task-per-PR workflow.
- **[T20260508-8]** — Resolve backend: cli subprocess cwd from workspace context and record it in audit/tracing.
- **[T20260509-2]** — Split the v2 job executor into responsibility-focused modules without changing runtime behavior.
- **[T20260509-7]** — Establish focused test coverage for the activity/job DAG executor (linear, retry, parallel, fan-out, loop, pipeline durability) and the macOS sandbox / policy boundary.
- **[T20260509-9]** — Auto-populate `task.context_files` from the winning planning-duel plan after resolution.
- **[T20260509-11]** — Keep condition guards on equality-only grammar and repair the `ship-auto` empty-backlog guard.
- **[T20260509-38]** — Run legacy parallel-batch workers through cancellable pipeline runs so timeout failure paths return promptly.
- **[T20260509-40]** — Run CLI subprocesses in killable process groups and bound timeout-path output reader joins.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
