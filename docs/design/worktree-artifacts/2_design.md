---
summary: "Worktree Artifacts - Design"
type: design
title: "Worktree Artifacts - Design"
owner: codex
last_updated: 2026-07-19
status: Accepted
feature: worktree-artifacts
doc_role: design
tags: ["worktree-artifacts"]
paths: ["crates/orbit-remote/**", "crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-cli/**"]
related_features: ["worktree-artifacts", "host-registry", "mcp-bridge"]
related_artifacts: ["ORB-00199", "ORB-00200", "ORB-00201", "ORB-10272", "ORB-10297", "ORB-10330", "ADR-0177", "ADR-0229"]
---

# Worktree Artifacts - Design

The current implementation treats ADR and learning bodies as branch-local files with globally allocated IDs. The shared root owns durable coordination state; the local root owns files that should be staged with the branch.

## 1. Runtime Roots

`OrbitRuntime` carries `shared_root` and `local_root`. On the main checkout they are equal. In a linked worktree, `shared_root` points to the main checkout `.orbit/`, and `local_root` points to the linked worktree `.orbit/`.

Explicit `--root` and `ORBIT_ROOT` overrides pin both roots to preserve the old single-root mental model when the operator asks for it.

## 2. Allocation Metadata

In standalone/worktree mode, `id_allocations` lives in
`shared_root/.orbit/state/semantic.db`. The allocator serializes ID creation with a
shared lock, then body writes update the row with:

- `worktree_root`: the recorded worktree root for the body.
- `branch`: best-effort current branch.
- `body_path`: the body file path relative to `worktree_root`.

Backfilled shared-root artifacts receive `body_path` during allocator initialization so old ADRs and migrated learnings remain readable from any worktree.

This remains the compatibility allocator and every current create path continues to
use it during F1. [ORB-10272] does not redirect standalone or worktree creation.

### 2.1 Hub-global sequence substrate

Multi-host authority is separate from worktree federation. Remote feature migration
v2 installs dormant, independent ADR and learning sequences in the hub's
config-resolved `orbit.db`, together with per-workspace reconciliation state and an
immutable `mcp_call_id` allocation ledger. Those rows are path-free; they neither
replace `body_path` nor make the hub a reader of a spoke owner's worktree.

Before hub authority can activate, every registered workspace's complete hub-local
legacy inventory is validated: all valid lifecycle files and all legacy allocation
rows in every status. Missing sources and cross-workspace duplicate IDs fail before
any mutation. The final forward-only reseed and authority flip are one restart-safe
transition. A late workspace stays knowledge-ineligible until the same complete
local reconciliation succeeds. The hub never contacts an owner to repair a missing
source.

After activation, allocation advances one kind's sequence and commits its immutable
correlation ledger plus canonical audit atomically. It still does not write the
artifact body: the owner writes the branch-local bundle under `local_root`. A
finalize failure therefore consumes a valid unused global ID. There is no
reservation, release, reuse, or remote-finalize protocol.

F1 leaves the substrate dormant and exposes no public allocation tool. F3 alone
activates public issuance and cuts owner creation over; standalone hosts cannot
enter hub authority.

### 2.2 Preallocated owner finalizers

[ORB-10330] adds the owner-side finalizer that consumes a hub allocation without
becoming an allocation authority. Each owner file store gains a
`finalize_preallocated(id, payload)` path beside its standalone create path: it
takes the caller-supplied canonical id (chosen upstream by the §2.1 hub
sequence), so it never calls the compatibility allocator, abandons, retries, or
selects a second id. It preserves the existing validation, exclusive bundle
creation, sidecar/index update, and local partial-write cleanup, and it installs
a **non-authoritative** owner-local body-path projection in the standalone
`id_allocations` table so ADR/learning list, show, and lifecycle resolve the
finalized body. The projection is inserted directly for the given id; it never
advances the local sequence and never claims canonical allocation authority.

A pre-existing artifact at the supplied id fails the finalization
deterministically and is never overwritten or adopted. A failure after the id is
fixed removes only the local partial bundle and projection; it never rolls back
or abandons the immutable hub allocation, which stays consumed as a valid gap.
The finalizer takes no absolute path — it operates on the D3-selected
checkout-bound owner store, so process cwd and remote paths cannot redirect it.

The composite `orbit.learning.add` / `orbit.adr.add` broker path pairs one hub
allocation with one owner finalization: for a local (hub-owned) workspace it
allocates through §2.1 then finalizes in the selected owner checkout; a foreign
spoke owner or a local replica is rejected by D3 owner preflight *before*
allocation, so no avoidable gap is burned. Allocation and owner finalization
correlate through the original trusted `mcp_call_id`, workspace id, kind, and
allocated id. [ORB-10330] adds and tests these finalizers and the broker
composition behind an inactive cutover gate; public creation stays on the
compatibility path until F3 activates issuance and cuts the callers over.

## 3. Write Path

ADR creation writes `adr.yaml` and `body.md` under `local_root/adrs/proposed/ADR-NNNN/`. Learning creation writes `learning.yaml`, `votes.jsonl`, and `comments.jsonl` under `local_root/learnings/L-NNNN/`.

The first write into a linked worktree creates only the subtree needed for the artifact type. It does not scaffold local `state/`, `audit/`, `tasks/`, scoreboards, or registry files.

## 4. Read Federation

ADR `show` resolves the envelope and body together in the store and carries exactly one of four states through Core, HTTP, and local MCP:

1. `local`: a structurally complete, non-empty bundle exists under the current local root. This wins over any readable sibling allocation, and the read does not rewrite the allocation row.
2. `federated`: no current-local bundle exists and the allocation resolves to a structurally complete, readable sibling bundle.
3. `remote_artifact_unavailable`: an allocation exists, but its `body_path`, `adr.yaml`, or `body.md` is absent, unreadable, inconsistent, or empty.
4. `not_found`: neither a current-local bundle nor an allocation exists.

Successful local and federated reads preserve the ADR envelope, return the exact non-empty `body`, and add:

```json
{
  "artifact_origin": {
    "mode": "local | federated",
    "worktree_root": "credential-safe path",
    "branch": "string or null"
  }
}
```

`mode` has no other values. Public origin payloads never include `body_path`, URLs, bearer material, or credentials. HTTP maps `remote_artifact_unavailable` to 409 while retaining its string `error`; local MCP uses the same stable code in its structured error. An unknown ID remains HTTP 404 / MCP `not_found`.

List and search retain their existing defaults. Readable allocation-owned bundles participate as before; unreadable rows are omitted unless `include_remote` requests the existing stub shape.

## 5. Mutation Boundary

ADR document update, accept, and supersede are local-only. A federated or unavailable allocation-owned artifact fails preflight with `artifact_not_local` (HTTP 409 or the same local MCP code) before any bundle, allocation, lifecycle timestamp, index, or audit mutation. Supersede preflights both operands before its first write. Landing the bundle in the current checkout restores ordinary local mutation semantics; a sibling-owned allocation row remains unchanged.

## 6. Indexing Behavior

Learning reindex and docs/ADR search operate on locally readable bodies. Remote-only allocation rows are skipped without error; once the recorded worktree is present and readable again, the same list/reindex path can read and index the body.

## 7. Concerns & Honest Limitations

Remote stubs are intentionally envelope-poor. They expose allocation metadata, not the artifact title, summary, or body, because those fields live in the unreadable body file. Filters that require body fields can only apply to locally readable artifacts.

The `worktree_root` column preserves historical rows from earlier phases, so old shared-root rows may record a `.orbit/` path while new rows record a worktree root. Readers resolve `body_path` relative to the recorded value instead of normalizing that history away.

## Task References

- [ORB-00199] introduced the runtime root split.
- [ORB-00200] introduced allocation metadata and the learning ID migration.
- [ORB-00201] implemented local body writes and read federation.
- [ORB-10297] made ADR federation body-preserving and typed the read/mutation boundary.
- [ORB-10272] added the dormant, path-free Remote-v2 hub sequence and reconciliation
  substrate while preserving the standalone shared-root allocator and owner-local
  body/federation semantics; F3 owns activation and caller cutover.
- [ORB-10330] added the owner-side preallocated finalizers (`finalize_preallocated`
  on the ADR and learning stores) and the gated broker composition: they consume a
  hub allocation into the exact owner checkout via a non-authoritative body-path
  projection, never allocate/abandon/retry, and reject replica/foreign-spoke owners
  before allocation. Public creation stays on the compatibility path until F3.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
