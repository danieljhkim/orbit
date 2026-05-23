---
summary: "Worktree Artifacts - Design"
type: design
title: "Worktree Artifacts - Design"
owner: codex
last_updated: 2026-05-20
status: Accepted
feature: worktree-artifacts
doc_role: design
tags: ["worktree-artifacts"]
paths: ["crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-cli/**"]
related_features: ["worktree-artifacts"]
related_artifacts: ["ORB-00199", "ORB-00200", "ORB-00201", "ADR-0177"]
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

List and show paths consult allocation metadata to resolve body files. If the recorded body path is readable, the store returns the full artifact from that path. If the body is missing or unreadable, default list output omits the row. `include_remote` includes a stub with ID, kind, allocation status, recorded worktree, branch, and body path.

`show` does not return a stub. A remote-only artifact is an error naming the recorded worktree and branch so the operator knows which worktree owns the body.

## 5. Indexing Behavior

Learning reindex and docs/ADR search operate on locally readable bodies. Remote-only allocation rows are skipped without error; once the recorded worktree is present and readable again, the same list/reindex path can read and index the body.

## 6. Concerns & Honest Limitations

Remote stubs are intentionally envelope-poor. They expose allocation metadata, not the artifact title, summary, or body, because those fields live in the unreadable body file. Filters that require body fields can only apply to locally readable artifacts.

The `worktree_root` column preserves historical rows from earlier phases, so old shared-root rows may record a `.orbit/` path while new rows record a worktree root. Readers resolve `body_path` relative to the recorded value instead of normalizing that history away.

## Task References

- [ORB-00199] introduced the runtime root split.
- [ORB-00200] introduced allocation metadata and the learning ID migration.
- [ORB-00201] implemented local body writes and read federation.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
