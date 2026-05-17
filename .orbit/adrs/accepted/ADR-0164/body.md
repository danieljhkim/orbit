## Context
`orbit run ship` reached PR-open Review handoff and PR-merge Done handoff through system-owned automation even when the workflow had a resolved implementer identity. Prior attribution fixes in ORB-00067, ORB-00089, and ORB-00091 covered adjacent automation paths, but the ship PR loops still had two real alternatives: trust the ship actor/runtime context, or carry task/run provenance explicitly.

## Decision
Ship-path PR transitions carry attribution on each automation update. The Review handoff uses existing `task.implemented_by`, then the pipeline resolved implementer identity, then task-authored fallback fields (`planned_by`, `created_by`), leaving the genuine actor-less fallback as `system`. The Done handoff preserves existing `implemented_by`, otherwise uses `created_by`, then `system`. Regression tests exercise PR-open review stamping and distinct Done identities in one batch so a batch-level actor cannot homogenize them.

## Consequences
- Shipped task records, ship scoreboards, and follow-on git author derivation can preserve the implementer family that actually produced each task before and after PR review.
- Actor-less automation still records `system` instead of panicking or fabricating a family label.
- Cost: the ship pipeline must explicitly bridge task/run provenance into automation update payloads, so future edits to PR-open or PR-merge loops need to preserve the regression tests rather than assuming runtime actor context is enough.