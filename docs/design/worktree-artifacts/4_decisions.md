---
summary: "Worktree Artifacts - Decisions"
type: design
title: "Worktree Artifacts - Decisions"
owner: codex
last_updated: 2026-07-19
status: Accepted
feature: worktree-artifacts
doc_role: decisions
tags: ["worktree-artifacts"]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-cli/**"]
related_features: ["worktree-artifacts", "host-registry", "mcp-bridge"]
related_artifacts: ["ORB-00199", "ORB-00200", "ORB-00201", "ORB-10272", "ORB-10297", "ORB-10330", "ADR-0177", "ADR-0229"]
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

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
