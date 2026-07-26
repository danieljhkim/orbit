---
title: Auto-tasks — Design
owner: claude
last_updated: 2026-07-26
status: Accepted
feature: auto-tasks
doc_role: design
type: design
summary: Current implementation of the auto-task record, due-math, host-local cursor, generic scheduler, CRUD surfaces, and the on-demand manual mint.
tags: [auto-tasks]
paths: ["crates/orbit-core/src/auto_tasks/**"]
related_features: [auto-tasks]
related_artifacts: [ORB-10149, ORB-10439, ADR-0218, ADR-0217]
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
last_task_id }`), using a file-locked read-modify-write.
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

## 5b. Manual mint — `generate` (ORB-10439)

`orbit auto-task generate <name>` mints one task from a definition on demand, so
a new or edited definition can be exercised without waiting for its slot (weekly
definitions otherwise cost a week per typo). It lives on `crud.rs` alongside the
other verbs and delegates to `scheduler::mint_task` — the scheduler's mint path
is already separable from due-math (it needs only the definition), so there is
exactly one template→task mapping and a generated task is field-for-field
identical to a fired one: same field mapping, same `auto-task:<name>` tag, same
`system_created` marker, same template-supplied status.

The mint is **unconditional**. It ignores schedule due-math, `dedupe`, and
`enabled`, and it neither reads nor writes the host-local cursor — an operator
naming a definition explicitly means it, and a manual mint must not perturb
scheduler state. Unknown names fail loudly (`InvalidInput` naming the
definition), so the CLI exits non-zero rather than silently no-op'ing.

Deliberately rejected:

- **A `generate`-local mint implementation.** A second template→task mapping
  would drift from the scheduler's, and the provenance parity that makes the
  feature worth having is precisely what drift destroys.
- **Honoring `enabled`/`dedupe`/due-math.** That makes `generate` a "run the
  scheduler early" button, which the existing `run_auto_task_scheduler` action
  already is. The gap being closed is *manual mint*, not *early fire*.
- **Advancing the cursor.** It would consume a real scheduled slot, silently
  cancelling the next automatic fire.
- **`--dry-run` / `--force` flags.** `--force` has nothing to override — the mint
  is already unconditional — and `--dry-run` would only re-print the template
  that `auto-task show` already renders. The surface is `<name>` plus `--json`,
  matching the sibling subcommands.

One consequence follows from parity and is intended: because a generated task
carries the provenance tag, an open one is visible to `skip_if_open` on the next
scheduler pass and defers that fire, exactly as an open fired instance does. The
cursor does not advance, so the deferred occurrence fires once when the queue
drains. This is the behavior the hand-copy workaround could not provide.

`generate` is CLI-only. Per the mcp-bridge design (`docs/design/mcp-bridge/2_design.md`,
auto_task placement row) the auto_task MCP tools manage the Git-versioned
definition and do not mint tasks; no MCP tool was added.

## 6. Concerns & Honest Limitations

The checked-in `qa-sweep` definition is the first concrete consumer. It files a
backlog task for crew `qa` every six hours, dedupes while one remains open, and
asks the executor to validate recent changes hands-on and file real findings
through Orbit. Its `no-diff-expected` tag lets workflow handoff succeed when the
validation correctly produces only task-side effects.

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
- ORB-10439 — `orbit auto-task generate <name>`, the on-demand manual mint.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
