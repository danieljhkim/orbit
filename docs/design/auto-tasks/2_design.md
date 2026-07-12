---
title: Auto-tasks — Design
owner: claude
last_updated: 2026-07-12
status: Accepted
feature: auto-tasks
doc_role: design
type: design
summary: Current implementation of the auto-task record, due-math, host-local cursor, generic scheduler, and CRUD surfaces.
tags: [auto-tasks]
paths: ["crates/orbit-core/src/auto_tasks/**"]
related_features: [auto-tasks]
related_artifacts: [ORB-10149, ADR-0218, ADR-0217]
---

# Auto-tasks — Design

This doc covers the shipped implementation: the definition record, discovery,
due computation, cursor state, the scheduler pass, and the CRUD surfaces. The
routine machinery it rides on (cron eval, fire records, dashboard health) is
documented under `docs/design/routines/`.

## 1. The definition record

`AutoTaskDefinition` (`crates/orbit-common/src/types/auto_task.rs`) is a
`deny_unknown_fields` struct: `schemaVersion`, `name`, `description`, `enabled`,
`schedule`, `template`, `dedupe`, and provenance (`created_by/at`,
`updated_by/at`). `schedule` is an untagged enum — `{ cron: "…" }` or
`{ every_minutes: N }`. `template` carries `title`, `description`,
`acceptance_criteria`, `task_type`, `tags`, `priority`, `crew`, and `status`
(default `backlog`). Per ADR-0217 there are **no turn-based knobs**; `deny_unknown_fields`
makes a stray `max_turns`/`turns` a hard parse error.

Definitions live as `.orbit/auto_tasks/<name>.yaml`. Discovery
(`loader.rs`) scans the directory, parses each file fail-closed, and rejects any
file whose stem ≠ its `name`, so the on-disk identity and the `auto-task:<name>`
provenance tag stay in lockstep.

## 2. Due computation and catch-up collapse

`schedule::decide_due(schedule, baseline, last_slot, now)` returns `NotDue` or
`Fire { slot }`. The effective exclusive floor is `last_slot` when the
definition has fired before, else `baseline` (its first-observed slot). Cron
reuses `routines::due::due_decision` under `MissedRunPolicy::CatchUpOnce`;
interval math jumps straight to the most recent boundary
`baseline + floor((now-baseline)/interval)·interval`. Either way a downtime gap
collapses to **one** fire, never one per missed slot.

## 3. Cursor state

`state.rs` stores one cursor per definition in
`<orbit_dir>/state/auto-tasks.json` (`{ baseline_at, last_slot, last_fired_at,
last_task_id }`), file-locked read-modify-write like the qa-sweep watermark.
This is workspace-local, gitignored runtime state (the scoreboard precedent,
L-0041), so a scheduler fire never rewrites the git-versioned definition and a
definition edit never races the scheduler.

## 4. The scheduler pass

`scheduler::run_auto_task_scheduler_at` loads the workspace's definitions and
cursors, then per enabled definition: on first sight it records a baseline and
fires nothing; otherwise it evaluates due-math. On `Fire`, if `dedupe =
skip_if_open` and a task tagged `auto-task:<name>` is still open, it skips
**without advancing the cursor** — so the pending occurrence fires (once,
collapsed) the moment the queue drains. Otherwise it mints a `system_created`
task from the template (tagged for provenance) and advances the cursor.

The pass is the deterministic `run_auto_task_scheduler` action
(`dispatch.rs`), wrapped in `auto_task_scheduler_pipeline` (`max_active_runs:
1`), fired by the seeded `auto_task_scheduler` routine (`overlap: forbid`,
minutely). Because it is a routine, its fires flow to `GET /api/routines`.

## 5. CRUD surfaces

`crud.rs` is the single choke point behind both the CLI (`orbit auto-task
add/list/show/update/toggle`) and the MCP tools (`orbit.auto_task.*`). Add
rejects duplicate names; update patches present fields; toggle flips `enabled`
(disabling is preserved, never a delete). Both surfaces validate the schedule
(cron parse / interval > 0) and crew at write time, so a bad definition is never
persisted.

## 6. Concerns & Honest Limitations

- **Definitions are not full-text indexed.** Unlike learnings/ADRs, auto-task
  YAML is not in a SQLite/search index; discovery is a directory scan. Acceptable
  at the expected cardinality (a handful of chores per workspace).
- **Workspace-scoped.** The scheduler processes the definitions of the workspace
  whose routine fired it, not a cross-workspace sweep. Multi-workspace fan-out is
  a future direction (see 3_vision.md).
- **Description secrets are not redacted** in the definition YAML (task creation
  still redacts when minting). Definitions are operator-authored, so this is
  low-risk, but not zero.

## Task References

- ORB-10149 — Auto-task primitive.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
