---
summary: "Worktree Artifacts - Decisions"
type: design
title: "Worktree Artifacts - Decisions"
owner: codex
last_updated: 2026-08-11
status: Accepted
feature: worktree-artifacts
doc_role: decisions
tags: ["worktree-artifacts"]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-engine/**", "crates/orbit-cli/**"]
related_features: ["worktree-artifacts", "host-registry", "mcp-bridge"]
related_artifacts: ["ORB-00199", "ORB-00200", "ORB-00201", "ORB-10272", "ORB-10297", "ORB-10330", "ORB-10501", "ORB-10535", "ORB-10545", "ORB-10668", "ORB-10669", "ORB-10725", "ADR-0177", "ADR-0229", "ADR-0296", "ADR-0302", "ADR-0339", "ADR-0342", "ADR-0357"]
---

# Worktree Artifacts - Decisions

ADR-style log for worktree artifact storage. Entries use repo-local ADR IDs ([ADR-0357]); this file is the record.

## ADR-0177 - Worktree-local ADR and learning bodies with shared ID allocation

**Status:** Accepted - 2026-05 - [ORB-00201]; amended 2026-07 by [ORB-10297], [ORB-10272], and [ORB-10330]; multi-host half withdrawn 2026-08 by [ORB-10725] under [ADR-0357]

**Context.** Linked worktrees need ADR and learning bodies committed with the code branch that created them, but IDs must remain collision-free across all worktrees. [ORB-00199] introduced shared/local root resolution and [ORB-00200] introduced the shared SQLite allocator. The remaining choice was whether body files follow the allocator into `shared_root` or follow the editing branch into `local_root`.

**Decision.** Write ADR and learning body files under the current worktree `local_root` while keeping standalone/worktree ID allocation, migration, and allocation metadata in `shared_root/.orbit/state/semantic.db`. Lists read through `id_allocations`: default output includes only locally readable bodies, while `include_remote` returns stubs that name the recorded worktree and branch. ADR show resolves envelope plus body once into `local`, `federated`, `remote_artifact_unavailable`, or `not_found`; a valid current-local bundle wins without rewriting allocation metadata. Successful show includes only the credential-safe `{mode, worktree_root, branch}` origin. ADR update, accept, and both supersede operands must be current-local before the first mutation and otherwise fail as `artifact_not_local`.

~~For multi-host mode, ADR-0229 adds a distinct hub-global authority rather than
stretching `semantic.db` federation across machines, installed dormant by Remote
feature migration v2 [ORB-10272] and consumed by the owner-side finalizers
[ORB-10330].~~ **Withdrawn** by [ADR-0357]: knowledge is keyed
`(workspace_id, artifact_key)`, there is no global allocator for any record type,
and [ORB-10725] deleted the substrate and the finalizers rather than parking them.
The worktree-federation decision above is unaffected — it was always
workspace-local — and `semantic.db` allocation remains what it describes.

**Consequences.**
- ADR and learning files can be staged in the same PR as the implementation that created them.
- Shared ID allocation still prevents cross-worktree collisions and records where each body lives.
- Workspace-local allocation is the whole story: an ID is unique within its
  workspace and two workspaces may both hold an `L-0007`. [ORB-10725]
- With no allocation call there is no reservation, expiry, orphaned ID, or
  finalize/pull race for a cross-machine create to get wrong. [ADR-0357]
- Readers get predictable defaults without failing on missing sibling-worktree files.
- Readable sibling ADRs preserve their exact body, while unavailable allocations and unknown IDs remain distinct typed failures.
- Rejected sibling-only mutations leave bundles, allocation metadata, lifecycle timestamps, and audit state unchanged.
- Cost: list/show paths now carry a federation boundary and must handle `body_path` metadata, remote stubs, and stale worktree paths.
- Cost: public HTTP, MCP, and CLI error mappers must preserve the origin discriminator and stable federation codes.
- Cost: because IDs now collide across workspaces by design, any merged
  cross-workspace read surface must carry the `workspace` field; a bare ID from
  such a result is not addressable. [ADR-0357]

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

## ADR-0302 - Publish superseded ADR bodies as durable decision history

**Status:** Accepted - 2026-08 - [ORB-10545]

Superseded ADR bundles, including their rejected alternatives and supersession
metadata, travel with the repository. Proposed drafts remain local-only. A
validated `orbit adr reconcile` operator path copies an existing complete
federated bundle byte-for-byte into the current registered checkout without
allocating a new ID or changing lifecycle/allocation metadata. Narrative and
the explicit rejected alternatives live in the ADR store; retrieve them with
`orbit tool run orbit.adr.show --input '{"id":"ADR-0302"}'`.

## ADR-0339 - Publish every ADR lifecycle partition and resolve duplicates by explicit precedence

**Status:** Proposed - 2026-08 - [ORB-10669]

Amends [ADR-0302]. Every ADR lifecycle partition — proposed included — travels
with the repository, so the decision under review is visible in the PR that
motivates it and a draft authored in a managed job worktree lands on that run's
branch instead of stranding on the box. Only the rebuildable index and the
host-local lock files stay ignored. The managed `.gitignore` block that
`orbit workspace init` writes carries the policy to every workspace and retires
the `proposed/` and `superseded/` ignore lines older blocks wrote, so re-init
converges instead of preserving the old policy. Publishing every partition makes
a duplicate ID reachable through an ordinary merge; it resolves by one explicit
precedence — the most-advanced lifecycle state wins — with each shadowed
partition named in a warning. Narrative and the explicit rejected alternatives
live in the ADR store; retrieve them with
`orbit tool run orbit.adr.show --input '{"id":"ADR-0339"}'`.

## ADR-0342 - orbit adr owns ADR authoring and lifecycle; reconcile stays the cross-checkout verb

**Status:** Proposed - 2026-08 - [ORB-10668]

`orbit adr` gains `add`, `update`, and `supersede`, each a thin delegation to the
matching `orbit.adr.*` tool, so an ADR authored in a managed job worktree can be
carried `proposed → accepted` from that worktree with the CLI alone. The tool
surface stays the single implementation of ADR semantics, and the §5 mutation
boundary is untouched: a non-local target still fails closed with
`artifact_not_local` and its `artifact_origin` payload. `orbit adr reconcile` is
*not* the answer for the in-owning-worktree case — the ADR is already local
there — and remains the verb for mutating an ADR from a checkout that does not
own it (§5.1, §6.1). The discoverability half is fixed as help text:
`orbit adr update --help` names the lifecycle transitions, the
`artifact_not_local` failure, and the reconcile escape hatch. Narrative and the
explicit rejected alternatives live in the ADR store; retrieve them with
`orbit tool run orbit.adr.show --input '{"id":"ADR-0342"}'`.

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
- [ORB-10545] added federated ADR reconciliation, published superseded bodies,
  and resolved the guarded-cleanup deadlock under [ADR-0302].
- [ORB-10669] published the remaining ADR partitions, made the shipped
  `.gitignore` block retire its own superseded lines so re-init converges, and
  replaced first-hit-wins resolution with the explicit lifecycle precedence
  under [ADR-0339].
- [ORB-10668] added the `orbit adr add` / `update` / `supersede` CLI verbs so the
  owning worktree can complete the lifecycle without `orbit tool run`, under
  [ADR-0342].

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
