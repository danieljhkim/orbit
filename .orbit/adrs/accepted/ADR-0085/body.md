## Context
Auto-refresh used `manifest.generated_at` versus `git log -1 --format=%cI` to decide if a clean branch graph was fresh. A branch reset, rebase, or old-date commit can move `HEAD` to a different checkout with a committer timestamp older than the manifest, causing reads to reuse a graph built for the previous checkout.

## Decision
Persist the build checkout's exact git identity on branch refs (`git_head_oid`, `git_tree_oid`) and mirror it in `manifest.json`. Clean-worktree refresh compares the current `HEAD` OID against the current branch ref before returning `Fresh`; tree OID remains a content fallback for partial records. Missing ref identity forces an incremental rebuild so newly written refs become self-describing. Commit timestamps remain diagnostic only.

## Consequences
- History rewrites, resets, and rebases refresh based on the actual checkout instead of wall-clock or commit dates.
- Branch refs become the per-branch freshness authority, which avoids treating a manifest from another branch as proof that the current branch is fresh.
- Legacy refs without identity rebuild once and then carry the new metadata.
- Cost: every build and clean refresh shells out to git for exact OIDs, adding a small fixed process cost to the refresh path.

---
