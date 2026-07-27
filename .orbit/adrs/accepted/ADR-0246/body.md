## Context
A task shipment can fail after an agent has produced coherent work but before the normal commit, rebase, push, and PR checkpoints finish. The real alternatives are to overload per-step recovery so it replays or impersonates later checkpoints, or give the workflow one explicit terminal failure hook that preserves the original failure while publishing any recoverable candidate.

## Decision
Add an optional job-level failure activity that runs once after a terminal step failure with the job input, completed pipeline checkpoints, failing step, and structured error. `task_pr_pipeline` binds it to a deterministic PR failure handoff which restores the pre-rebase candidate, commits dirty work, pushes without rewriting unknown remote history, opens or reuses a blocked/manual-resolution PR, and blocks the task while the original run still terminalizes as failed.

## Consequences
- Normal success and retry checkpoints remain unchanged; the failure handoff is an explicit, auditable last-chance side effect.
- A conflict-blocked run is distinguishable through its task status event, PR body, and failure-activity audit even though the original step failure remains authoritative.
- Cost: JobV2 gains another lifecycle hook and task shipment maintains a dedicated deterministic recovery action with conservative Git/remote rules.
- Cost: External push or PR service outages can still prevent publication, but dirty work is committed locally before those fallible operations so terminal runs do not strand uncommitted changes.