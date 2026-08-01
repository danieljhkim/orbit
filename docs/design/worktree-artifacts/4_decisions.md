---
summary: "Worktree Artifacts - Decisions"
type: design
title: "Worktree Artifacts - Decisions"
owner: codex
last_updated: 2026-08-01
status: Accepted
feature: worktree-artifacts
doc_role: decisions
tags: ["worktree-artifacts"]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-engine/**", "crates/orbit-cli/**"]
related_features: ["worktree-artifacts", "host-registry", "mcp-bridge"]
related_artifacts: ["ORB-00199", "ORB-00200", "ORB-00201", "ORB-10272", "ORB-10297", "ORB-10330", "ORB-10501", "ORB-10535", "ADR-0177", "ADR-0229", "ADR-0296"]
---

# Worktree Artifacts - Decisions

ADR-style log for worktree artifact storage. Entries use globally allocated ADR IDs; the corresponding `.orbit/adrs/...` artifact is the source of truth for lifecycle metadata.

## ADR-0177 - Worktree-local ADR and learning bodies with shared ID allocation

**Status:** Accepted - 2026-05 - [ORB-00201]; amended 2026-07 by [ORB-10297], [ORB-10272], and [ORB-10330]

**Context.** Linked worktrees need ADR and learning bodies committed with the code branch that created them, but IDs must remain collision-free across all worktrees. [ORB-00199] introduced shared/local root resolution and [ORB-00200] introduced the shared SQLite allocator. The remaining choice was whether body files follow the allocator into `shared_root` or follow the editing branch into `local_root`.

**Decision.** Write ADR and learning body files under the current worktree `local_root` while keeping standalone/worktree ID allocation, migration, and allocation metadata in `shared_root/.orbit/state/semantic.db`. Lists read through `id_allocations`: default output includes only locally readable bodies, while `include_remote` returns stubs that name the recorded worktree and branch. ADR show resolves envelope plus body once into `local`, `federated`, `remote_artifact_unavailable`, or `not_found`; a valid current-local bundle wins without rewriting allocation metadata. Successful show includes only the credential-safe `{mode, worktree_root, branch}` origin. ADR update, accept, and both supersede operands must be current-local before the first mutation and otherwise fail as `artifact_not_local`.

For multi-host mode, ADR-0229 adds a distinct hub-global authority rather than
stretching `semantic.db` federation across machines. Remote feature migration v2
installs its dormant, path-free ADR/learning sequences, validated reconciliation
state, and immutable correlation ledger in the hub `orbit.db` [ORB-10272]. Sequence
advance, ledger append, and audit are atomic; owner-local body creation remains a
separate step and may leave a valid gap. F1 preserves every existing allocator and
create caller. F3 alone activates and cuts public issuance over. The hub never
proxies to or reads a spoke owner's worktree.

**Consequences.**
- ADR and learning files can be staged in the same PR as the implementation that created them.
- Shared ID allocation still prevents cross-worktree collisions and records where each body lives.
- Hub-global allocation is reconciled above every workspace's complete legacy
  file/allocation maximum without turning worktree paths into protocol data.
- A late workspace is explicitly ineligible until its complete hub-local inventory
  reconciles; missing sources and duplicate IDs fail before mutation.
- Standalone/worktree allocation remains unchanged until the explicit F3 cutover.
- Owner finalization of a hub id installs a non-authoritative body-path projection
  only; it never chooses an id, advances a local sequence, or claims allocation
  authority, and a finalize failure leaves the hub allocation consumed as a valid
  gap while removing every local partial. [ORB-10330]
- Readers get predictable defaults without failing on missing sibling-worktree files.
- Readable sibling ADRs preserve their exact body, while unavailable allocations and unknown IDs remain distinct typed failures.
- Rejected sibling-only mutations leave bundles, allocation metadata, lifecycle timestamps, and audit state unchanged.
- Cost: list/show paths now carry a federation boundary and must handle `body_path` metadata, remote stubs, and stale worktree paths.
- Cost: public HTTP, MCP, and CLI error mappers must preserve the origin discriminator and stable federation codes.

## ADR-0296 - Detect and retire id allocations pinned to a reaped worktree

**Status:** Accepted - 2026-07 - [ORB-10501]; amended 2026-08 by [ORB-10535]

**Context.** ADR-0177 pins each allocated id to the worktree that wrote its body, and models a body that is not readable here as a *remote stub* — which assumes the body still exists in some other checkout. That assumption has no steady state. When a job-run worktree is reaped before its body was finalized and merged, the allocation row outlives every path that could resolve it: the row stays visible as `reserved`/`merged` forever, the body is unrecoverable, and nothing detects or prunes it. F2026-07-161 measured 35 of 113 allocated learning ids in `ws_orbit` as unreadable remote stubs (17 `reserved`, 18 `merged`), several pinned to worktrees confirmed gone from disk; the same pattern hit ADR-0149, 0181, 0194, 0211, and 0225. `learning sync` cannot help — it reconciles only from locally readable YAML (F2026-07-094 b). The allocator's `abandon_learning`/`abandon_adr` existed but were reachable only from create-rollback and refuse any row that recorded a body path, which is exactly what a stranded `merged` row has.

**Decision.** An allocation is **orphaned** when both hold: its pinned `worktree_root` no longer exists on disk, *and* its body is unreadable both canonically and through the recorded `body_path`. Both are required — a live sibling worktree is an ordinary remote stub, and a canonically present body makes a stale `worktree_root` harmless. Before automated cleanup removes a worktree, the shared removal path reads the live allocation rows under the allocator lock and refuses when a body pinned to the target has no byte-identical readable copy in another registered worktree. This preflight applies to forced pipeline cleanup and ordinary GC alike and reports the affected IDs with reconciliation instructions [ORB-10535].

Orphans that already exist are reported by the `id-allocations` `orbit doctor` check and retired by the guarded `orbit doctor --fix-orphaned-allocations`. Repair flips `status` to `abandoned` rather than deleting the row: `max_sequence` counts abandoned rows, so a retired id is never reissued, and the row keeps its recorded worktree, branch, and `body_path` for forensics. The allocator repair entry point differs from the create-rollback `abandon` in accepting a `merged` row and in guarding on the missing worktree, re-checked inside the write transaction; the owning store re-verifies both orphan conditions immediately before each write, so a caller working from a stale scan cannot retire a recoverable id. Learning repair additionally drops the stale envelope index row pinned to the same dead body.

**Consequences.**
- The permanently-orphaned class is detectable rather than inferred by hand-reading `learning list --include-remote`, and repairable without hand-editing `.orbit/` or the SQLite store.
- Retired ids stay consumed, so repair can never collide with an id already cited in a commit message or a doc.
- Deleting the row was rejected: `max_sequence` would reissue the id, and the only remaining record of where the body was written would be destroyed.
- Repair is opt-in behind an explicit flag; the check only warns, and an ordinary `orbit doctor` run mutates nothing.
- Automated cleanup now fails closed before data loss, while a body already landed byte-for-byte in the canonical or another registered checkout remains eligible for cleanup.
- The prevention guard and ORB-10501 repair remain separate: bodyless reservations and already-missing worktrees are still doctor concerns rather than cleanup-time repairs.
- Cost: the orphan test is duplicated per artifact kind (two ~25 LOC store methods) because ADR and learning bodies resolve differently; lifting it into the allocator would push artifact-layout knowledge down into the id authority, which is the boundary ADR-0177 draws.
- Cost: `worktree_root.exists()` is a liveness heuristic — a worktree on an unmounted volume reads as reaped. The refuse-on-recoverable guard and the opt-in flag bound the blast radius to a status flip that never touches a body file.

## Task References

- [ORB-00199] introduced shared/local root resolution.
- [ORB-00200] introduced shared ID allocation and `L-NNNN`.
- [ORB-00201] implemented this decision.
- [ORB-10297] amended the ADR show and mutation boundary with four-state resolution and typed origin/error payloads.
- [ORB-10272] amended the allocation boundary with the dormant Remote-v2 hub-global
  sequence, full legacy reconciliation, immutable correlation ledger and atomic
  audit while retaining standalone compatibility and owner-local bodies.
- [ORB-10330] added the owner-side preallocated finalizers and the gated broker
  composition that consume a hub allocation into the exact owner checkout — one
  hub allocation, one owner finalization, correlated by `mcp_call_id`, with
  replica/foreign-spoke rejection before allocation and no local sequence advance.
- [ORB-10501] added detection and guarded repair for allocations whose pinned
  worktree was reaped, closing the steady-state gap the remote-stub model left
  open.
- [ORB-10535] added the shared pre-removal guard that prevents cleanup from
  creating that orphaned state when the target still holds the unique body.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
