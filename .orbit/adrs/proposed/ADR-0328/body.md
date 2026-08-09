## Context
A parent shipment previously treated every failed review child identically, so a pre-review infrastructure failure triggered the same blocked/manual-reconciliation handoff as a reviewer rejection. The alternatives were to keep generic child-status gating, weaken the worktree guard, or make the review boundary classify whether a durable reviewer checkpoint exists.

## Decision
Keep the worktree integrity guard unchanged and require review dispatch to supply the complete workspace_path/repo_root pair. At the parent boundary, classify a failed child with no durable reviewer checkpoint as review-not-started, preserve the child diagnostic in the parent failure, and make terminal PR handoff record that event without blocking or republishing the already-published candidate. A child with a reviewer checkpoint, including request_changes, remains a review-ran failure and retains the ordinary blocked handoff.

## Consequences
- Operators can distinguish infrastructure startup failure from a reviewer verdict in the parent run and task history without opening the child run.
- Review-not-started still fails the gated parent, but the task and existing PR stay in review for a clean retry rather than implying that code reconciliation was requested.
- The worktree guard remains fail-closed; safety comes from threading and testing the complete declared path pair for every supported reviewer provider family.
- Cost: the generic pipeline success guard now carries an opt-in review contract and depends on durable child step checkpoints to classify review progress.