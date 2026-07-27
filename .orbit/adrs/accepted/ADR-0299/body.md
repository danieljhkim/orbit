## Context
Orbit admitted provider-created commits after external Stop hooks could move an assigned worktree HEAD, which duplicated history, trailer, task-scope, and adoption policy across the provider boundary and commit phase. The alternatives were to retain narrow commit adoption, add another compatibility layer, or restore a single workflow-owned committer while preserving the independent dirty-work recovery and process-scoped attribution decisions.

## Decision
Providers may edit assigned worktree files but must not create commits or otherwise move the assigned HEAD or branch; the provider boundary rejects every such movement with a typed integrity diagnostic. `commit_batch_changes` compares HEAD directly with the immutable setup SHA, stages the worktree diff, and creates exactly one workflow-owned commit without traversing or adopting provider history, parsing provenance trailers, or proving paths against task context. Dirty integrity failures retain the run-keyed tracked patch, untracked payload, and manifest. Workflow commits retain the persisted crew-model author and process-scoped Orbit committer established compatibly by ADR-0279 and ADR-0280, without exporting hook-specific commit-message state.

## Consequences
- A successful implementation leaves only worktree changes for the workflow commit phase, and that phase returns the one SHA it creates.
- Provider-created commits fail at the provider boundary and are never inspected for admission or adopted downstream.
- Dirty integrity failures remain byte-for-byte recoverable after forced linked-worktree cleanup.
- Git authorship remains attributable to the persisted crew model, the committer remains process-scoped, and durable task/run records carry workflow provenance without mandatory Git trailers.
- Cost: a legitimate manual or provider-side commit in an assigned worktree is rejected even when its contents and attribution could have been proven safe; recovery requires returning the candidate to an uncommitted worktree diff before rerunning the workflow.