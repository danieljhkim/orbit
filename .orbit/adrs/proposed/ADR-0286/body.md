## Context
Auto-task definition CRUD used the shared Orbit root even when a workflow executor was assigned a linked worktree. That let a refresh expose tracked dirt in the registered primary checkout while unrelated implementation guards were sampling it. The alternatives were to tolerate auto_tasks drift in the integrity guard or to make tracked definition mutation worktree-local.

## Decision
Read and replace tracked auto-task definitions through the runtime local root. Keep scheduler cursors and coordination state under the shared root, and replace each definition atomically so a failed refresh cannot expose partial bytes.

## Consequences
- Linked-worktree refreshes become ordinary branch changes and concurrent implementation guards continue to observe an unchanged primary checkout.
- Primary-checkout operator commands retain existing behavior because local and shared roots are identical there.
- Cost: callers in linked worktrees see that checkout version of definition YAML while scheduler cursor state remains shared, so definition and cursor roots must remain deliberately separate.