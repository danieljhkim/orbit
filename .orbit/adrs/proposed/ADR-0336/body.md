## Context

A resumed PR pipeline could report successful delivery against a base branch that had already merged and been deleted. The base name flows `input.base_branch -> prepare_branch.output.base -> sync_base.output.base -> pr_open.base` and is never re-derived, and `resolve_worktree_start_point` is satisfied by any `origin/<base>` that resolves — a leftover or restored branch resolves to its pre-merge tip. `open_or_reuse_pr` validated only local divergence of head against the pinned base sha (`branch_freshness_against_ref`) and then handed the base name straight to PR creation. Every step reported success, and the resulting merge did not put the commit on the landing branch. This is the failure mode hardest to notice, because every signal says the work landed.

The question "is this work actually merged into that commit" was already answered once, for dependencies, in `worktree::dependency_delivery` (ADR-0290): match the `[ORB-NNNNN]` marker every Orbit commit message carries, because squash and rebase rewrite the sha and preserve the message.

## Decision

A base branch is **obsolete** when it can no longer carry work to the branch that work lands on, by either of two tests:

1. **Deleted** — the repository has an `origin` remote and the base branch is gone from it. A PR cannot merge into a branch the remote does not have, so a local or stale remote-tracking ref that still resolves is a leftover, not a target. Always on; it costs one `ls-remote`, which the pipeline already pays for the head branch in `pr_prepare`.
2. **Already landed** — a `landing_branch` input is declared, differs from the base, and the base carries nothing the landing branch does not already have: either the pinned base sha is an ancestor of the landing tip (merge / fast-forward), or every commit unique to the base is already delivered on the landing branch under its task marker (squash / rebase — the shape Orbit's own `merge_batch_pr` produces).

Test 2 reuses `vcs::delivery_marker`, lifted out of `worktree::dependency_delivery` so both gates share one marker rule rather than two that can drift. The pinned `base_sha` is the subject of both tests (ADR-0251, L-0113); only the landing branch is resolved live, because its current tip is exactly what the question is about.

The gate runs at `pr_open` (refuse to create or reuse a PR) and again at `pr_promote` (a resume can enter the pipeline there, with a PR opened before its base landed). `input.base_obsolescence='ignore'` is the escape hatch, mirroring `dependency_delivery`.

Rejected alternatives: probing the base branch's own PR through GitHub (couples the gate to the API, and a local-history rule verifies PR-backed and local-only bases identically); defaulting `landing_branch` to the repository default branch (an integration branch fully merged into `main` right after a release promotion would then be read as obsolete and every ordinary delivery refused).

## Consequences

- The silent failure becomes a loud, phase-labeled refusal (`[phase=obsolete-base]`) naming the stale base, the landing branch, the marker that already landed, and a recovery path.
- Ordinary non-stacked delivery is unchanged: with no `landing_branch`, or with it equal to the base, only the remote-existence probe runs.
- The obsolescence half of the gate is opt-in by declaration. Orbit has no durable notion of a landing branch, and inventing a default is the one change that could refuse healthy production delivery. Stacked dispatchers must set `landing_branch`; without it the pipeline is no worse off than before.
- Cost: two false-positive classes, both escapable with `base_obsolescence='ignore'` — a live base whose only unique commits repeat an already-landed task id (a task re-opened and re-run), and a base deliberately kept off `origin` while an `origin` remote exists.