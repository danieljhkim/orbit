## Context

Learning and ADR ids come from one shared SQLite allocator and are pinned to the worktree that allocated them, with the body written into that worktree (ADR-0177). Lists model a body that is not readable here as a *remote stub*, which assumes the body still exists in some other checkout.

That assumption has no steady state. When a job-run worktree is reaped before its body was finalized and merged, the allocation row outlives every path that could resolve it: the row stays visible as `reserved`/`merged` forever, the body is unrecoverable, and nothing detects or prunes it. F2026-07-161 measured 35 of 113 allocated learning ids in `ws_orbit` as unreadable remote stubs (17 `reserved`, 18 `merged`), several pinned to worktrees confirmed gone from disk; the same pattern hit ADR-0149, 0181, 0194, 0211, and 0225. `learning sync` cannot help — it reconciles only from locally readable YAML (F2026-07-094 b).

The allocator already had `abandon_learning`/`abandon_adr`, but they were reachable only from create-rollback and refuse any row that recorded a body path, which is exactly what a stranded `merged` row has.

## Decision

Define an **orphaned allocation** as a row satisfying both conditions: its pinned `worktree_root` no longer exists on disk, *and* its body is unreadable both canonically and through the recorded `body_path`. Both are required — a live sibling worktree is an ordinary remote stub, and a canonically present body makes a stale `worktree_root` harmless.

Report orphans through a new `id-allocations` `orbit doctor` check, and retire them with the guarded `orbit doctor --fix-orphaned-allocations`. Repair flips `status` to `abandoned` rather than deleting the row: `max_sequence` counts abandoned rows, so a retired id is never reissued, and the row keeps its recorded worktree, branch, and `body_path` for forensics. The new allocator entry point differs from the create-rollback `abandon` in accepting a `merged` row and in guarding on the missing worktree, re-checked inside the write transaction. The owning store re-verifies both orphan conditions immediately before each write, so a caller working from a stale scan cannot retire a recoverable id. Learning repair additionally drops the stale envelope index row, which is pinned to the same dead body.

## Consequences

- The permanently-orphaned class is detectable rather than inferred by hand-reading `learning list --include-remote`, and repairable without hand-editing `.orbit/` or the SQLite store.
- Retired ids stay consumed, so repair can never cause a collision with an id that was cited in a commit message or a doc.
- Deleting the row was rejected: it would let `max_sequence` reissue the id, and would destroy the only remaining record of where the body was written.
- Repair is opt-in behind an explicit flag; the check itself only warns, and an ordinary `orbit doctor` run mutates nothing.
- Cost: the orphan test is duplicated per artifact kind (two ~25 LOC store methods) because ADR and learning bodies resolve differently; lifting it into the allocator would push artifact layout knowledge down into the id authority, which is the boundary ADR-0177 draws.
- Cost: `worktree_root.exists()` is a liveness heuristic — a worktree on an unmounted volume reads as reaped. The refuse-on-recoverable guard and the opt-in flag bound the blast radius to a status flip that never touches a body file.