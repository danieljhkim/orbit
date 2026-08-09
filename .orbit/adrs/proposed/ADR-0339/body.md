## Context

ADR-0302 (ORB-10545) published superseded ADR bundles but kept `proposed/`
local-only, continuing ORB-10303. Two costs followed. The decision under review
was invisible in the PR that motivated it, so a reviewer could not read the
draft alongside the change. And every ADR authored inside a managed job-run
worktree was stranded on the box: the draft lived in an ignored directory that
died with the worktree unless an operator ran `orbit adr reconcile` first.

The ignore policy is generated, not hand-maintained. `ORBIT_GITIGNORE_BLOCK` in
`crates/orbit-cli/src/command/workspace/support.rs` is the managed block that
`orbit workspace init` writes and rewrites into every workspace, and it still
ignored both `.orbit/adrs/proposed/` and `.orbit/adrs/superseded/` — the latter
already contradicting ADR-0302. A hand-edited `.gitignore` was therefore
reverted on the next init or re-register.

Tracking the proposed partition also makes a latent ambiguity reachable.
Acceptance is a directory rename from `proposed/<id>` to `accepted/<id>`. With
both partitions tracked, a branch cut before acceptance still carries the
proposed bundle; merging it re-adds that directory next to the accepted one, and
because the two paths are unrelated git merges both without a conflict.
`locate_adr` resolved by scanning `AdrStateDir::all()` in declaration order —
proposed first — and returning the first hit with no duplicate detection, so the
stale draft would mask the accepted record and the ADR would silently read as
proposed again.

## Decision

Publish every ADR lifecycle partition. `proposed/`, `accepted/`, `superseded/`,
and `deleted/` all travel with the repository; only the rebuildable
`adrs/index.sqlite*` and the host-local `*.lock` files stay ignored. The managed
block carries this to every workspace and additionally *retires* the two ignore
lines that older blocks wrote. Retirement is load-bearing rather than cosmetic:
`!.orbit/adrs/` re-includes only the `adrs` directory itself, so a surviving
`.orbit/adrs/proposed/` above the appended block would still be the last pattern
matching that subdirectory and would keep the partition ignored. Stripping it is
what makes re-init converge on the current policy instead of preserving the old
one, without duplicating or stacking blocks.

Resolve a duplicated ID by one explicit, documented precedence: the
most-advanced lifecycle state wins, ranked `proposed` < `accepted` <
`superseded` < `deleted`. This is sound because every sanctioned transition
moves forward and `accepted -> proposed` is rejected outright, so the
lower-ranked copy is always the stale one. `AdrStateDir::lifecycle_rank` states
the ranking; `AdrStateDir::all()` is documented as scan order carrying no
resolution meaning. `locate_adr` collects every partition hit before choosing,
`list_adrs` collapses duplicates under the same rule so a stale draft cannot
double-count in a listing or race the accepted row into an index rebuild, and
each shadowed partition is named with its path in a `warn` log so the leftover
is observable and removable.

`orbit adr reconcile` keeps its stricter contract: a source checkout holding
more than one lifecycle artifact for an ID is refused, not resolved. Federated
reconciliation, artifact ownership, and the `artifact_not_local` guard are
unchanged — this decision governs publication, not ownership.

Drafts written in a managed job-run worktree get a defined disposition: they are
tracked, so the run's auto-commit sweeps them onto that branch and they ride the
PR. A run that is abandoned or rejected takes its draft with the branch — no
operator cleanup, no reconciliation, and the unused ID allocation is an ordinary
valid gap, not the orphaned-allocation condition ORB-10501 repairs.

### Rejected alternatives

- **Fail the read on a duplicate.** An error is more obviously deterministic,
  but it bricks `orbit adr show` at exactly the moment an operator needs it —
  immediately after a merge — and offers no path forward except manual
  filesystem surgery. Precedence plus a warning is deterministic *and*
  recoverable, and it still surfaces the leftover. Reconcile keeps the strict
  behavior where the operand set is under operator control.
- **Reorder `AdrStateDir::all()` so the accepted partition scans first.** This
  fixes the one reachable pair by moving the implicitness rather than removing
  it; the next reader still cannot tell that declaration order is load-bearing,
  which is the defect.
- **Sweep proposed drafts out of job worktrees before commit.** Deleting drafts
  in the run's worktree would destroy exactly the artifact publication is meant
  to preserve, and cannot distinguish an abandoned run from one whose PR is
  about to merge.
- **Only remove the two lines from the block, without a retirement list.**
  Existing checkouts would keep their old lines ahead of the appended block and
  never converge, so every already-initialized workspace would silently retain
  the old policy.

## Consequences

- A proposed ADR is reviewable in the PR that motivates it, and an ADR authored
  in a managed worktree lands on its own branch without an operator step.
- The shipped block now matches the accepted publication policy for every
  partition, including the `superseded/` line ADR-0302 had already invalidated.
- Re-init over a checkout carrying an older managed block converges on the
  current policy exactly once, with no stacked or duplicated block.
- A merged stale draft can no longer mask an accepted record on read, in a
  listing, or in an index rebuild, and it is reported rather than silent.
- Cost: `list_adrs` now accumulates through a keyed map instead of appending
  during the walk, so its output is ordered by ID rather than by partition scan
  order. Callers that need another order already sort explicitly.
- Cost: drafts from abandoned runs accumulate as dead objects in unmerged branch
  history. They are unreachable once the branch is deleted, but they are not
  actively pruned.
- Cost: the precedence is a repair, not a prevention. A duplicate still reaches
  the working tree on merge, and clearing the warning means deleting the stale
  directory by hand.