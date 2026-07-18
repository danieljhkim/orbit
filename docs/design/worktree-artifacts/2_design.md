---
summary: "Worktree Artifacts - Design"
type: design
title: "Worktree Artifacts - Design"
owner: codex
last_updated: 2026-07-18
status: Accepted
feature: worktree-artifacts
doc_role: design
tags: ["worktree-artifacts"]
paths: ["crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-cli/**"]
related_features: ["worktree-artifacts"]
related_artifacts: ["ORB-00199", "ORB-00200", "ORB-00201", "ORB-10297", "ADR-0177"]
---

# Worktree Artifacts - Design

The current implementation treats ADR and learning bodies as branch-local files with globally allocated IDs. The shared root owns durable coordination state; the local root owns files that should be staged with the branch.

## 1. Runtime Roots

`OrbitRuntime` carries `shared_root` and `local_root`. On the main checkout they are equal. In a linked worktree, `shared_root` points to the main checkout `.orbit/`, and `local_root` points to the linked worktree `.orbit/`.

Explicit `--root` and `ORBIT_ROOT` overrides pin both roots to preserve the old single-root mental model when the operator asks for it.

## 2. Allocation Metadata

`id_allocations` lives in `shared_root/.orbit/state/semantic.db`. The allocator serializes ID creation with a shared lock, then body writes update the row with:

- `worktree_root`: the recorded worktree root for the body.
- `branch`: best-effort current branch.
- `body_path`: the body file path relative to `worktree_root`.

Backfilled shared-root artifacts receive `body_path` during allocator initialization so old ADRs and migrated learnings remain readable from any worktree.

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

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
