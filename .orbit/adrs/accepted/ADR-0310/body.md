## Context

Orbit records the crew that executes a task and the token and cost facts for managed agent invocations, but it does not record which orchestration crew is accountable for selecting, sequencing, and shepherding that task. Inferring orchestration ownership from `created_by`, `implemented_by`, execution `crew`, or a job actor conflates different responsibilities and prevents meaningful orchestration performance comparisons. Existing invocation-to-task linkage is many-to-many, so summing per-task metrics would also double-count shared invocations. Direct interactive Codex or Claude session cost is not present in the managed invocation ledger and is intentionally outside this first increment.

## Decision

Add an optional task field named `orchestrator` containing an exact registered crew alias such as `sol`, `terra`, `opus`, or `sonnet`. It is distinct from the execution `crew`, provider family, model id, and session id. Authors set it explicitly; Orbit does not infer or backfill it for legacy or system-generated tasks. Explicit writes are validated against the same registered crew namespace used by execution crews, while reads tolerate historical aliases that are no longer configured.

The field may be assigned or corrected while a task is `proposed` or `backlog`, and becomes immutable when execution starts. This preserves stable historical attribution without introducing temporal handoff reconstruction in v1. The field is optional in task bundle schema version 1; old bundles deserialize to `None`, and the forward-compatibility limitation for older binaries is documented.

Managed execution metrics are computed from distinct invocation records, never by summing per-task aggregates. Each invocation is classified exactly once: `missing` when any linked task cannot be resolved; `unattributed` when it has no linked task or any resolved task lacks an orchestrator; a named orchestrator when all linked tasks resolve to the same orchestrator; or `shared` when all resolve but name multiple orchestrators. The aggregate exposes all token splits and separate provider-cost, derived-cost, comparable-cost, and unknown-cost populations under an exclusive `as_of` cutoff. Its reconciliation invariant is that bucket invocation counts and accounting facts equal the distinct source invocation population for the requested window.

The dashboard exposes orchestration metrics as a separate dimension from executor-agent metrics. Direct interactive orchestration-session cost remains a future, separate telemetry lane and is not allocated across tasks in v1. Existing ADR-0245 remains the authority for query-time price derivation.

## Consequences

- Orbit can compare managed execution spend and outcomes by accountable orchestration crew without conflating that crew with the executor.
- Legacy, unowned, partially attributable, cross-orchestrator, and missing-task invocations remain visible rather than being guessed into a named bucket.
- Task ownership is deliberately frozen once work starts; a future design is required for mid-execution orchestration handoffs.
- Direct Codex and Claude orchestration-session overhead is excluded from the initial metric and must be instrumented independently later.
- Cost: task creation and update surfaces, bundle persistence, validation, invocation accounting, API, dashboard, fixtures, and compatibility documentation all require coordinated changes.