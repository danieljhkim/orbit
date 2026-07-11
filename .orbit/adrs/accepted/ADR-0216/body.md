## Context
The unattended triage agent (ORB-10129) must never free-hand task lifecycle transitions, yet its per-task verdicts have to reach task state somehow. Real alternatives: reuse the existing `update_task` deterministic activity fed from agent output (but it accepts arbitrary statuses, so a hallucinated `done` would be applied verbatim), or let the agent call `orbit.task.update` through its tool allowlist (no deterministic bound at all).

## Decision
A dedicated `apply_triage_dispositions` deterministic action is the only lifecycle writer in `task_triage_pipeline`. It cross-checks every disposition against the deterministic candidate list from `list_triage_candidates`, restricts the vocabulary to `rebacklog`/`stay_blocked`, honors `rebacklog` only for the `environmental` classification, enforces the durable re-backlog budget by counting `triage_rebacklogged` history events, and re-checks task status + run coupling before each write so overlapping runs skip instead of double-transitioning. The `steps.triage.output.dispositions` handoff is the allowlisted exception in `default_jobs_template_only_declared_agent_loop_handoffs`.

## Consequences
- The agent's output is advisory data: it cannot move a task anywhere but `backlog`, cannot touch a task outside the candidate list, and cannot exceed the budget — misclassification degrades to a wasted bounded retry, never a corrupted lifecycle.
- Loop-guard state lives in task history events rather than a new task field, so exhaustion survives restarts and is human-auditable.
- Cost: a second bespoke deterministic action (and its activity asset) to maintain instead of reusing `update_task`, and the disposition vocabulary must be extended in Rust when triage learns new outcomes.