---
summary: "Worktree Artifacts - Overview"
type: design
title: "Worktree Artifacts - Overview"
owner: codex
last_updated: 2026-05-20
status: Accepted
feature: worktree-artifacts
doc_role: overview
tags: ["worktree-artifacts"]
paths: ["crates/orbit-core/**", "crates/orbit-store/**", "crates/orbit-cli/**"]
related_features: ["worktree-artifacts"]
related_artifacts: ["ORB-00199", "ORB-00200", "ORB-00201", "ADR-0177"]
---

# Worktree Artifacts - Overview

Worktree artifacts let ADR and learning body files travel with the branch that created them while preserving one shared ID authority for the whole repository. Tasks, audit, scoreboards, and allocator state stay in the shared `.orbit/`; ADR and learning bodies live in the current worktree's `.orbit/`.

## 1. Motivation

Linked git worktrees let agents work on several branches at once, but the old single-root artifact model wrote every ADR and learning body into the main checkout. That made a branch's code change and its knowledge artifacts land in different working trees, so agents could not stage the full change together.

The three-task sequence split this apart:

- [ORB-00199] exposed `shared_root` and `local_root`.
- [ORB-00200] moved ADR and learning ID allocation into a shared SQLite allocator and migrated learnings to `L-NNNN`.
- [ORB-00201] writes ADR and learning bodies into `local_root` and reads them through allocation metadata.

## 2. Core Concepts

| Concept | Meaning |
|---------|---------|
| Shared root | The main checkout `.orbit/`, used for tasks, audit, scoreboards, semantic.db, and allocation authority. |
| Local root | The current worktree `.orbit/`, used for ADR and learning body files. |
| Allocation row | A row in `id_allocations` recording ID, kind, allocation status, recorded worktree, branch, and `body_path`. |
| Local-readable artifact | An allocation whose recorded body path exists and can be read by the current process. |
| Remote stub | A list row for an allocation whose body is not locally readable, shown only with `include_remote`. |

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Root split | `crates/orbit-core/src/runtime/resolve.rs` | [ORB-00199] |
| Allocator and body metadata | `crates/orbit-store/src/sqlite/id_allocator.rs` | [ORB-00200], [ORB-00201] |
| ADR body storage and federation | `crates/orbit-store/src/file/adr_store/api.rs` | [ORB-00201] |
| Learning body storage and federation | `crates/orbit-store/src/file/learning_store/api/crud.rs` | [ORB-00201] |
| CLI/tool remote listing | `crates/orbit-core/src/runtime/orbit_tool_host/` and `crates/orbit-cli/src/command/learning/` | [ORB-00201] |
| Decision log | `docs/design/worktree-artifacts/4_decisions.md` | [ADR-0177] |

## Task References

- [ORB-00199] split Orbit runtime resolution into shared and local roots.
- [ORB-00200] introduced the global ADR/Learning allocator and `L-NNNN` learning IDs.
- [ORB-00201] moved ADR/Learning body writes to the current worktree and added read federation.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
