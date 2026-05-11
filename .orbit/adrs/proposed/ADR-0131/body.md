## Context
Three plausible designs for "where does the team's task state live?" exist: a coordinator daemon (shared-host), per-host ID suffixes that paper over allocation collisions without a shared store, and a git-based registry. v1 commits to per-engineer deployment ([README](../../../README.md), [POSITIONING](../../POSITIONING.md)), which rules out a coordinator daemon. Per-host suffixes (`T20260504-h7a3-1`) preserve uniqueness but complicate the local commit-message search convention, audit events, and downstream tooling. Knowledge-graph task attribution was removed in [T20260506-11] and is no longer a current consumer. A git-based orphan-branch registry preserves the ID format and uses infrastructure the team already has.

## Decision
The task registry lives on a git orphan branch at `refs/heads/orbit/tasks` (user-facing name `orbit/tasks`) on the team's shared remote. Every sync-enabled mutation fetches this ref, mutates locally, commits on the branch, and pushes. Atomic git ref update is the coordinator. Reject coordinator daemon: it would violate the v1 per-engineer doctrine and reintroduce the shared-host work that v1 explicitly defers. Reject per-host suffixes: they break ID-format-as-interface across the system.

## Consequences
- Sync inherits the team's existing git auth, transport, and ACL.
- The branch is inspectable with standard `git log` and `git diff` tooling.
- The choice of `refs/heads/orbit/tasks` (over `refs/orbit/tasks`) means branch protection, code review tools, and host UIs all recognize the ref without custom config.
- Cost: every sync-enabled mutation requires a network roundtrip. Workspaces that need offline `task add` must keep sync disabled or use the explicit `--offline` escape hatch.

---
