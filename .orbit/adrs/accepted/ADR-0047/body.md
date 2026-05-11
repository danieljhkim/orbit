## Context
Task records already store `implemented_by`, but automated `git_commit` actions previously delegated commit authorship to local git config, hiding the agent that actually produced the change.

## Decision
Pass a per-commit `--author` derived from `task.implemented_by` for single-implementer commits. Mixed-implementer batch commits use `orbit <orbit@orbit.local>` as the aggregate author and add one `Co-Authored-By` trailer per distinct implementer identity. ADR-023 extends this provenance to committer identity without reusing repo-local user config.

## Consequences
- Reviewers can see implementation provenance directly in git history without joining back through run audit events.
- Local git config is not written by workflow commit automation and is no longer the source of committer identity for those commits.
- Cost: multi-implementer batch commits require trailer-aware attribution queries; `git log --author` finds the aggregate commit author, not every co-author trailer.

---
