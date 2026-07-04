## Context

The original sketch allowed a `run: {type: shell, command: ...}` payload for small chores. ADR-0194 removed the `shell` activity variant and `run_shell` dispatch fail-closed; reintroducing arbitrary-command payloads through the scheduler would reopen that surface on a timer, unattended.

## Decision

`target:` accepts only catalog references resolved at load time; unresolvable targets are load-time errors. v1 dispatches `job:<name>` — run dispatch is job-shaped (`submit_pipeline_run` resolves jobs by name; there is no standalone activity run entrypoint), so `activity:<name>` is reserved and rejected at parse time with guidance to wrap the activity in a one-step job in the same source workspace. Shell-like chores become `deterministic` activities or jobs in the source workspace.

## Consequences

- Scheduled execution inherits existing activity/job policy, audit envelopes, and the fail-closed posture of ADR-0194; the scheduler adds a trigger source, not a new execution surface.
- Load-time validation makes a broken reference visible on the next sweep instead of at fire time.
- Cost: every new chore requires authoring a catalog asset (higher friction than a one-line command), and scheduler capability is permanently coupled to catalog capability — including the job-shaped dispatch constraint that keeps `activity:` targets out of v1.