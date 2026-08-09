## Context
ADR-0302 published accepted, superseded and deleted bundles but kept proposed drafts local-only, reasoning that an unaccepted draft is not yet decision history. In practice the proposed partition is where every ADR authored inside a managed job worktree first lands, so the decision under review is invisible in the pull request that motivates it, review happens against a bundle only the box can read, and promotion requires an operator reconcile step. The real alternative was to keep drafts ignored and treat reconciliation as the publication path, accepting the review blind spot as the price of a history free of abandoned drafts.

## Decision
All four ADR state partitions — proposed, accepted, superseded and deleted — are tracked and travel with the repository; only the rebuildable SQLite index and lock files stay ignored. The managed `.gitignore` block written by `orbit workspace init` is the single expression of that policy for every workspace, not a per-checkout edit. Because a tracked draft can be re-added by a merge after acceptance, ADR resolution must no longer let a stale `proposed/` bundle mask a more advanced state for the same ID.

## Consequences
- A proposed ADR is reviewable in the change that introduces it, and promotion to accepted is an ordinary tracked rename rather than an on-box operator step.
- Reconciliation from ADR-0302 remains the mechanism for adopting a federated bundle into another checkout, but publication no longer depends on it.
- Duplicate-ID resolution becomes load-bearing rather than unreachable: resolution currently returns the first match scanning proposed-first, so lifecycle precedence must be made explicit or a duplicate must be surfaced as an error.
- Drafts written inside managed job worktrees become tracked files in the run's branch, so abandoned and rejected runs can leave proposed bundles behind and need a defined disposition.
- Cost: the repository accumulates decisions that were never accepted, including drafts from failed runs, and readers must treat `proposed/` as a slush pile rather than as decisions the project stands behind.