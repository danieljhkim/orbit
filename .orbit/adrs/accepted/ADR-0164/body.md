## Context
`orbit run ship` reached the Review -> Done transition through system-owned automation even when each task already carried agent provenance. Prior attribution fixes in ORB-00067, ORB-00089, and ORB-00091 covered adjacent automation paths, but the batch PR merge loop still had two real alternatives: trust the ship actor/runtime context, or carry each task provenance explicitly.

## Decision
Ship-path Done transitions use per-task provenance as the source of truth: `task.implemented_by` wins, then `task.created_by`, then the genuine actor-less fallback remains `system`. The merge loop passes that resolved value on the task update for each task, and the regression test exercises distinct identities in one batch so a batch-level author cannot homogenize them.

## Consequences
- Shipped task records, ship scoreboards, and follow-on git author derivation can preserve the implementer family that actually produced each task.
- Actor-less automation still records `system` instead of panicking or fabricating a family label.
- Cost: the ship pipeline must explicitly bridge task provenance into the automation update payload, so future edits to that loop need to preserve the regression test rather than assuming runtime actor context is enough.