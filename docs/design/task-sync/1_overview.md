# Task Sync — Overview

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-06

Task sync is an opt-in, git-orphan-branch task registry that lets engineers on the same team see each other's tasks without running a shared Orbit instance. **Sync is a v2 feature.** v1 ships per-engineer per the [README](../../../README.md) and [POSITIONING](../../POSITIONING.md) doctrines — each operator runs Orbit on their own machine, with tasks, locks, and audit DB local to that machine. This document captures the v2 design now while context is fresh; the implementation lands as a separate sequence of tasks once the design is Accepted.

**2026-05-06 note ([T20260506-11]).** The prior cross-machine `task_id` motivation has been retired for v1. Orbit task IDs remain local search keys for the author (`git log --grep '[T...]'`); cross-engineer task references go through `external_refs`. This task-sync design remains a future v2 registry proposal, not a statement that today's `task_id` is globally resolvable.

This document is the entry point. [2_design.md](./2_design.md) specifies the mechanism, call sites, and migration paths in detail; [3_vision.md](./3_vision.md) names open questions and prior work; [4_decisions.md](./4_decisions.md) is the ADR log.

---

## 1. Motivation

The per-engineer-deployment commitment is honest about what v1 supports, but it leaves a visible coordination gap. Engineer A creates [T20260504-1] on their laptop; engineer B has no way to know it exists short of asking, finding the resulting PR, or reading commits the agent produces. For a 10–50 engineer team, that's expensive friction — exactly the friction the per-engineer framing was supposed to be honest about, not pretend away.

Three obvious options for closing the gap:

1. **A shared Orbit server.** Solves it natively but is the v2 shared-host story, not v1.
2. **Sync the task store via some external mechanism.** Generic, vague, hand-wavy.
3. **Use git itself as the coordination primitive.** The team already has a shared git remote. Auth, ACL, and transport are already solved by whatever git host the team uses.

Option 3 is what task sync proposes. A single orphan branch (proposed: `orbit/tasks`, ref `refs/heads/orbit/tasks`) holds the canonical task registry. `orbit task add` and mutating `orbit task update` paths fetch this ref before mutating, write locally, commit on the orphan branch, and push. Atomic git ref update is the coordinator. No new server, no new auth surface, no break to the v1 deployment doctrine.

The name "task sync" is deliberately narrow. It means task YAML bundles plus their companion files (`plan.md`, `execution-summary.md`, `artifacts/**`). It does *not* mean syncing locks, the audit DB, scoreboards, or job runs. Those have different consistency needs and are out of scope at any version — see [2_design.md §7](./2_design.md).

---

## 2. Core Concepts

### 2.1 Task registry

The orphan branch acting as the canonical store of task bundles. The ref is `refs/heads/orbit/tasks`, the user-facing branch name is `orbit/tasks`. Its tree mirrors the workspace `.orbit/tasks/` layout exactly: status directories at the top (`proposed/`, `backlog/`, `in_progress/`, `review/`, `done/`, `blocked/`, `archived/`, `rejected/`, `friction/`, `someday/`), date-partitioned subdirectories (`<yyyy-mm>/`) under terminal states, and a per-task directory containing `task.yaml`, `plan.md`, `execution-summary.md`, and an optional `artifacts/` subtree.

Why an orphan branch and not a normal feature branch: the registry has no shared history with code branches, and merging it into `main` would be nonsensical. Orphan history keeps the registry inspectable via standard `git log refs/heads/orbit/tasks` while preventing accidental merge into product branches.

### 2.2 Online-mode mutation

When sync is enabled in a workspace, `orbit task add` and mutating `orbit task update` paths become online-only: they require a working git remote, fetch the registry ref before allocating IDs or writing bundles, and push afterward. Read paths (`orbit task list`, `orbit task show`) remain offline-capable because the workspace's local task tree is always materialized.

The design's choice to make mutations online-only is what makes git's atomic ref update load-bearing. Without it, two engineers could race their local stores and produce divergent histories that no merge strategy can reconcile cleanly.

### 2.3 Conflict resolution

The hard problem. Standard git text-merge fails for task bundles in three concrete ways:

- **Status transitions** move task YAMLs between status directories. Two engineers transitioning the same task to different states produce two on-branch paths for the same task — git accepts both, leaving the task in two states.
- **Comments and history** are append-only YAML lists. Concurrent appends in different commits text-merge into structurally damaged YAML.
- **Concurrent same-field edits** surface as text conflicts that humans cannot resolve usefully because YAML quoting and indentation are hostile.

Task sync v2 ships **operation-aware replay**: on push reject, the client re-fetches, replays the operation against the new tip rather than text-merging, and pushes again. Most operations replay automatically; concurrent edits to the same field surface as explicit, structured conflicts the user can resolve with a `task sync resolve` step. Three other options were considered and rejected — see [2_design.md §3](./2_design.md) for the full comparison and [4_decisions.md ADR-002](./4_decisions.md).

### 2.4 v1/v2 boundary

This design exists in v1 as a docs-only artifact. v1 ships with `[task.sync] enabled = false` as the default and no sync code. The implementation lives behind that flag and lands incrementally in v2. The decision to defer is itself an ADR ([4_decisions.md ADR-001](./4_decisions.md)) because it has costs the team should be able to read and challenge.

---

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Folder layout, frontmatter, ADR template | [docs/design/CONVENTIONS.md](../CONVENTIONS.md) | — |
| Per-engineer deployment doctrine | [README.md](../../../README.md), [POSITIONING.md](../../POSITIONING.md) | — |
| Task ID allocation (`T<YYYYMMDD>-<N>`) | [crates/orbit-store/src/file/task_store/layout.rs:102-132](../../../crates/orbit-store/src/file/task_store/layout.rs) | [T20260505-12] |
| Task bundle write path | [crates/orbit-store/src/file/task_store/bundle.rs](../../../crates/orbit-store/src/file/task_store/bundle.rs) | [T20260505-12] |
| Task store API (create/update/delete) | [crates/orbit-store/src/file/task_store/api.rs](../../../crates/orbit-store/src/file/task_store/api.rs) | [T20260505-12] |
| Task scoping strategy (`WorkspaceOnly`) | [crates/orbit-store/src/scope.rs](../../../crates/orbit-store/src/scope.rs) | [T20260505-12] |
| Per-machine reservation locks (out of scope) | [crates/orbit-store/src/sqlite/task_reservation_store.rs](../../../crates/orbit-store/src/sqlite/task_reservation_store.rs) | — |
| Conflict-resolution mechanism comparison | [2_design.md §3](./2_design.md) | [T20260505-12] |
| ADR log | [4_decisions.md](./4_decisions.md) | [T20260505-12] |
| Open questions, prior work | [3_vision.md](./3_vision.md) | [T20260505-12] |

---

## Task References

- [T20260505-12] — Design git-orphan-branch task sync (v2 feature). The task that produced this folder.
- [T20260421-0528] — Historical knowledge-graph task attribution work. Superseded as evidence that `T<YYYYMMDD>-<N>` must be load-bearing across machines by [T20260506-11].

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
