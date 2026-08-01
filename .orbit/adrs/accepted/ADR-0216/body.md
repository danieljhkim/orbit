## Context
The unattended triage agent (ORB-10129) normally returns advisory dispositions to a bounded deterministic writer. ORB-10243 exposed two cases the original design did not cover: work can already be verifiably merged when a later pipeline step fails, and a `stay_blocked` diagnosis for the same coupled failed run otherwise causes a fresh agent diagnosis on every sweep. The alternatives were to keep all reconciliation human-only, or to add a new agent-output disposition applied by the deterministic step; both retain unnecessary repeat work or require trusting advisory output to represent evidence gathered outside that step.

## Decision
`apply_triage_dispositions` remains the only writer for disposition-driven transitions, with its candidates-only, still-blocked, same-coupled-run, environmental-only re-backlog, and durable-budget bounds unchanged. The triage agent has one narrow direct-write exception: for a listed blocked candidate whose own deliverable is conclusively evidenced as merged to the landing branch, it may call `orbit.task.update` to move that candidate from `blocked` to `done` and attach the exact merge evidence. It then returns `stay_blocked`; the deterministic apply step re-reads the task, observes that it is no longer blocked, and skips without a second write.

`list_triage_candidates` suppresses implicit re-triage when the task history already contains a `triage_diagnosis` naming the currently coupled failed run. A different coupled `run_id` is new evidence and remains eligible, while explicit `task_ids` input bypasses suppression for human-requested re-diagnosis.

## Consequences
- Agent output remains advisory; the sole direct lifecycle permission is an evidence-gated `blocked` → `done` write on a candidate the deterministic listing supplied.
- Externally completed work is reconciled without re-running merged work, and the existing still-blocked apply guard provides overlap safety and idempotency.
- Same-run `stay_blocked` verdicts await human action without recurring agent cost, while new failures and explicit requests remain visible.
- Cost: the shipped triage instruction and tool allowlist now carry one auditable lifecycle exception, and same-run suppression depends on the stable `triage_diagnosis` history-note prefix.