---
summary: "Task Sync — Overview"
type: design
title: "Task Sync — Overview"
owner: claude
last_updated: 2026-07-02
status: Superseded
feature: task-sync
doc_role: overview
tags: ["task-sync"]
---

# Task Sync — Overview

> **Superseded (ORB-00029, ORB-00030, 2026-07).** This git-orphan-branch *task-sync* design is retired and was never built. The cross-machine task-visibility gap it targeted is now addressed by live remote/multi-workspace dashboard viewing — `orbit web serve --global` and `orbit web connect <ssh-host> [--global]` — documented under [`docs/design/remote-access/`](../../remote-access/1_overview.md). See ADR-0200 (why viewing supersedes a git-sync registry) and ADR-0201 (SSH-tunnel-over-loopback transport). This folder is retained only as the historical record of the rejected git-sync mechanism; nothing here ships. Note the tradeoff ADR-0200 names: viewing is *not* sync — it has no offline, writable, or cross-machine-merge story, so a team that genuinely needs a shared writable registry would have to revisit this design.

Task sync is an opt-in, git-orphan-branch task registry that lets engineers on the same team see each other's tasks without running a shared Orbit instance. **Sync is a future feature.** Orbit still ships per-engineer per the [README](../../../README.md) and [POSITIONING](../../POSITIONING.md) doctrines — each operator runs Orbit on their own machine, with locks and audit DB local to that machine. This document captures the sync design on top of the current v2 task-artifact store: canonical task bundles live under `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/`, workspace checkouts expose `.orbit/tasks/<task-id>` projections, and task IDs use `ORB-00000`. Historical `T...` task references in this folder are archival references, not current sync-design inputs.

This document is the entry point. [2_design.md](./2_design.md) specifies the mechanism, call sites, and migration paths in detail; [3_vision.md](./3_vision.md) names open questions and prior work; [4_decisions.md](./4_decisions.md) is the ADR log.

---

## 1. Motivation

The per-engineer-deployment commitment is honest about what Orbit supports, but it leaves a visible coordination gap. Engineer A creates `ORB-00042` on their laptop; engineer B has no way to know it exists short of asking, finding the resulting PR, or reading commits the agent produces. For a 10–50 engineer team, that's expensive friction — exactly the friction the per-engineer framing was supposed to be honest about, not pretend away.

Three obvious options for closing the gap:

1. **A shared Orbit server.** Solves it natively but is the shared-host story, not the local product shape.
2. **Sync the task store via some external mechanism.** Generic, vague, hand-wavy.
3. **Use git itself as the coordination primitive.** The team already has a shared git remote. Auth, ACL, and transport are already solved by whatever git host the team uses.

Option 3 is what task sync proposes. A single orphan branch (proposed: `orbit/tasks`, ref `refs/heads/orbit/tasks`) holds the canonical task registry. `orbit task add` and mutating `orbit task update` paths fetch this ref before mutating, write locally, commit on the orphan branch, and push. Atomic git ref update is the coordinator. No new server, no new auth surface, no break to the per-engineer deployment doctrine.

The name "task sync" is deliberately narrow. It means task YAML bundles plus their companion files (`description.md`, `acceptance.md`, `plan.md`, `execution-summary.md`, `events.jsonl`, `comments.jsonl`, `review-threads/**`, and `artifacts/**`). It does *not* mean syncing locks, the audit DB, scoreboards, or job runs. Those have different consistency needs and are out of scope at any version — see [2_design.md §7](./2_design.md).

---

## 2. Core Concepts

### 2.1 Task registry

The orphan branch acting as the shared registry of task bundles. The ref is `refs/heads/orbit/tasks`, the user-facing branch name is `orbit/tasks`. Its tree mirrors the v2 canonical bundle shape, not the local symlink projection: `workspaces/<workspace-id>/<task-id>/` contains `task.yaml`, Markdown sidecars, JSONL logs, review-thread files, and artifacts.

Why an orphan branch and not a normal feature branch: the registry has no shared history with code branches, and merging it into `main` would be nonsensical. Orphan history keeps the registry inspectable via standard `git log refs/heads/orbit/tasks` while preventing accidental merge into product branches.

### 2.2 Online-mode mutation

When sync is enabled in a workspace, `orbit task add` and mutating `orbit task update` paths become online-only: they require a working git remote, fetch the registry ref before allocating IDs or writing bundles, and push afterward. Read paths (`orbit task list`, `orbit task show`) remain offline-capable because the workspace's local task tree is always materialized.

The design's choice to make mutations online-only is what makes git's atomic ref update load-bearing. Without it, two engineers could race their local stores and produce divergent histories that no merge strategy can reconcile cleanly.

### 2.3 Conflict resolution

The hard problem. Standard git text-merge fails for task bundles in three concrete ways:

- **Status transitions** update `task.yaml` and append `events.jsonl`. Two engineers transitioning the same task to different states can both produce syntactically valid commits with incompatible lifecycle intent.
- **Comments and history** are append-only JSONL streams. Concurrent appends can converge, but only when replayed as operations rather than line-level text merges.
- **Concurrent same-field edits** surface as text conflicts that humans cannot resolve usefully because YAML quoting and indentation are hostile.

Task sync uses **operation-aware replay**: on push reject, the client re-fetches, replays the operation against the new tip rather than text-merging, and pushes again. Most operations replay automatically; concurrent edits to the same field surface as explicit, structured conflicts the user can resolve with a `task sync resolve` step. Three other options were considered and rejected — see [2_design.md §3](./2_design.md) for the full comparison and [4_decisions.md ADR-002](./4_decisions.md).

### 2.4 Implementation Boundary

This design is still docs-only. Orbit ships with local v2 task artifacts but no task-sync coordinator. When sync lands, it should build on the current registry-backed task store rather than reintroducing status directories or `T...` IDs. The decision to defer is itself an ADR ([4_decisions.md ADR-001](./4_decisions.md)) because it has costs the team should be able to read and challenge.

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Folder layout, frontmatter, ADR template | [docs/design/CONVENTIONS.md](../CONVENTIONS.md) | — |
| Per-engineer deployment doctrine | [README.md](../../../README.md), [POSITIONING.md](../../POSITIONING.md) | — |
| Task ID allocation (`ORB-00000`) and workspace binding | [crates/orbit-store/src/sqlite/task_registry.rs](../../../crates/orbit-store/src/sqlite/task_registry.rs) | — |
| Task bundle read/write path | [crates/orbit-store/src/file/task_store/v2_bundle.rs](../../../crates/orbit-store/src/file/task_store/v2_bundle.rs), [v2_store.rs](../../../crates/orbit-store/src/file/task_store/v2_store.rs) | — |
| Task bundle storage contract | [task-artifacts/specs/task-bundle-v2.md](../task-artifacts/specs/task-bundle-v2.md) | — |
| Task scoping strategy (`WorkspaceOnly`) | [crates/orbit-store/src/scope.rs](../../../crates/orbit-store/src/scope.rs) | [T20260505-12] |
| Per-machine reservation locks (out of scope) | [crates/orbit-store/src/sqlite/task_reservation_store.rs](../../../crates/orbit-store/src/sqlite/task_reservation_store.rs) | — |
| Conflict-resolution mechanism comparison | [2_design.md §3](./2_design.md) | [T20260505-12] |
| ADR log | [4_decisions.md](./4_decisions.md) | [T20260505-12] |
| Open questions, prior work | [3_vision.md](./3_vision.md) | [T20260505-12] |

---

## Task References

- [T20260505-12] — Original git-orphan-branch task sync proposal. Historical reference; the design now targets the `ORB-*` task-artifact shape.
- [T20260421-0528] — Historical knowledge-graph task attribution work. Superseded as evidence that old task IDs must be load-bearing across machines by [T20260506-11].

Resolve archival task references with `git log --grep=<ID>`; new Orbit tasks use `ORB-*` IDs.
