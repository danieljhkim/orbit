## Context
The seeded task workflows added many small ADRs as shipment behavior grew: run aliases, deterministic auto-dispatch, remote base selection, recovery hooks, backlog exclusions, operator status, friction admission, and lock cleanup. They are one decision family: task shipment is an explicit durable workflow, not an advisory agent step or hidden side effect.

## Decision
Keep `orbit run` workflow aliases focused on execution, make automatic task shipment deterministic from backlog listing through gate fan-out, default shipping worktrees to fetched remote base refs, admit tasks through status-aware workflow gates, and protect overlapping work with durable task-lock reservations whose seeded TTL covers the child wait budget. Recovery is bounded, step-scoped on direct shipment workflows, and assigned through the configured reviewer role; child pipeline joins are followed by deterministic success guards after required cleanup, operator status is derived from persisted pipeline state, accepted friction reports enter auto-backlog by `status: backlog`, and run-owned reservations clean up when their owner run reaches a terminal state.

Folded instances:

| ADR | Instance folded into this rollup |
|-----|----------------------------------|
| ADR-014 | Public run workflows are execution aliases only. |
| ADR-024 | Shipping worktrees default to fetched remote base refs. |
| ADR-028 | Job-level recovery handles retry-exhausted step errors. |
| ADR-029 | The first direct-shipment recovery default was deterministic and conservative. |
| ADR-030 | Default recovery is step-scoped and agent-driven. |
| ADR-033 | Auto-backlog lock exclusions are structured output. |
| ADR-034 | `ship-auto` reports operator workflow status from durable pipeline state. |
| ADR-035 | Gate reservations release after terminal child waits. |
| ADR-037 | Accepted friction reports enter auto-backlog by status. |
| ADR-039 | Run-owned task-lock reservations clean up at owner terminal. |

## Consequences
- Task shipment workflows expose durable admission, recovery, status, and lock state without asking downstream steps to parse model output.
- Auto-dispatch no longer depends on provider credentials before it has deterministic backlog bundles.
- Gate-owned reservations serialize overlapping bundles while their owner run is alive and are released by both seeded early-release steps and engine-owned terminal cleanup.
- Seeded gate defaults require `ttl_seconds >= dispatch_timeout_seconds` so a legal child shipment wait cannot outlive its admission reservation.
- Costs retained from folded entries:
- Cost: the auto-dispatch audit trail no longer contains a model-authored advisory grouping note.
- Cost: users of `orbit run ship local`, `orbit run ship list/show`, and `orbit run duel list/show` must update their command muscle memory and scripts.
- Cost: default shipping workflows now require the configured base branch to be fetchable from `origin`; callers that intentionally operate without a remote must opt into `base_sync: local`.
- Cost: job authors must make the recovery activity generic enough for every retryable step in that job.
- Cost: this is intentionally conservative; it does not perform semantic git cleanup, task mutation, or child-run reconciliation until a more specific recovery policy is justified.
- Cost: default recovery now depends on the configured reviewer agent being available, and authors must decide which steps deserve recovery rather than flipping one workflow-level switch.
- Cost: the Rust serializer and seeded activity YAML schema now duplicate the exclusion shape and must be kept in sync.
- Cost: the CLI formatter now knows selected fields from `task_auto_pipeline` state, so future pipeline key renames must either preserve compatibility or update the operator summary parser.
- Cost: `task_gate_pipeline` now relies on the dynamic `task_{{ input.mode }}_pipeline` job-name convention, so future gate modes must either follow that naming convention or refactor the dispatch selector.
- Cost: child dispatch status remains data until explicit guard steps run, so seeded workflow authors must preserve guard placement after cleanup when they fork task-shipment YAML.
- Cost: longer default gate reservations can block overlapping work for up to two hours if both explicit release and run-owned cleanup fail.
- Cost: reviewers must read friction eligibility as a status rule, not a task-type rule.
- Cost: job-run finalization and reservation reserve paths are more coupled, so new terminal run paths must route through the cleanup helper rather than writing directly to the job-run store.
