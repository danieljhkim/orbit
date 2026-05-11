## Context
Reusing local Git config for workflow committers made agent identities sticky in developer repositories. If `user.name` or `user.email` was set to an agent identity in repo-local config, later human commits inherited that attribution.

## Decision
Automated `git_commit` actions set author and committer identity only for the spawned `git commit` process. Single-implementer commits use that implementer's scoped identity for both author and committer. Mixed-implementer commits use `orbit <orbit@orbit.local>` as the aggregate author and committer while preserving distinct implementers as `Co-Authored-By` trailers. Workflows must not write agent or aggregate identities into repo-local Git config.

## Consequences
- Human `user.name` and `user.email` settings remain byte-for-byte stable across workflow commits.
- Worktrees with no local `user.*` config can still create workflow-owned commits with explicit provenance.
- The public `git.commit` tool remains user-directed and ambient-config based; workflow-owned commit automation uses this scoped path instead.

---
