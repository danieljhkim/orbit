---
title: Auto-tasks — Overview
owner: claude
last_updated: 2026-07-26
status: Accepted
feature: auto-tasks
doc_role: overview
type: design
summary: Dynamically-defined recurring task templates minted by one generic scheduler routine — periodic work as data, not code.
tags: [auto-tasks]
paths: ["crates/orbit-core/src/auto_tasks/**"]
related_features: [auto-tasks]
related_artifacts: [ORB-10149, ORB-10318, ORB-10348, ORB-10439, ORB-10446, ADR-0218, ADR-0217]
---

# Auto-tasks — Overview

Auto-tasks turn recurring chores into **data instead of code**. An auto-task
definition is a git-versioned YAML record with a schedule, an `enabled` toggle,
a task template, and a dedupe policy. One generic scheduler routine reads the
enabled definitions, fires the due ones, and mints a task from each template.
Adding a new periodic chore is a new definition (`orbit auto-task add`), never
new orbit code or a new routine.

## 1. Motivation

Every periodic need in orbit used to be bespoke: qa-sweep (ORB-10039) was an
inline check sweep, ship-sweep had its own systemd unit, and each future
recurring chore would mean another hardcoded routine. The marginal cost of a new
chore was a code change, a review, and a release. Auto-tasks replace the pattern
with a primitive so the marginal cost is a YAML record. qa-sweep V1 (ORB-10148)
becomes just the first definition.

## 2. Core Concepts

- **Definition** — a `.orbit/auto_tasks/<name>.yaml` record; `name` is identity.
  Modeled on the file-backed routine convention (directory scan, fail-closed
  parse, `deny_unknown_fields`), not the SQLite-indexed learning/ADR convention.
- **Schedule (cadence)** — a 5-field `cron` or an `every_minutes` interval, in
  the definition's `schedule` field. Catch-up always collapses: a downtime gap
  mints one make-up task, not one per slot. Cadence is per-definition data, not
  a knob in the identity `config.yaml` ([L-0014] keeps runtime config out of
  `config.yaml`).
- **Cursor** — per-definition last-fired state, host-local at
  `<orbit_dir>/state/auto-tasks.json`, so the git-versioned definition is never
  churned by a scheduler fire.
- **Scheduler** — one deterministic activity (`run_auto_task_scheduler`) wrapped
  in the `auto_task_scheduler_pipeline` job, fired by the seeded
  `auto_task_scheduler` routine. Its fires appear on the dashboard routines
  surface.
- **Dedupe & provenance** — each minted task carries an `auto-task:<name>` tag;
  `skip_if_open` uses that tag to avoid firing while a prior instance is open.
- **Manual mint** — `orbit auto-task mint <name>` mints one task from a
  definition immediately, reusing the scheduler's mint path so the result is
  indistinguishable from a fired instance. Unconditional and cursor-inert: it
  ignores schedule, `dedupe`, and `enabled`, and never touches
  `<orbit_dir>/state/auto-tasks.json` (ORB-10439).

## 3. At a Glance

| Concern | File | Task |
|---|---|---|
| Definition schema | `crates/orbit-common/src/types/auto_task.rs` | ORB-10149 |
| Discovery (fail-closed) | `crates/orbit-core/src/auto_tasks/loader.rs` | ORB-10149 |
| Due-math + catch-up | `crates/orbit-core/src/auto_tasks/schedule.rs` | ORB-10149 |
| Host-local cursor | `crates/orbit-core/src/auto_tasks/state.rs` | ORB-10149 |
| Scheduler pass | `crates/orbit-core/src/auto_tasks/scheduler.rs` | ORB-10149 |
| CRUD (CLI + MCP shared) | `crates/orbit-core/src/auto_tasks/crud.rs` | ORB-10149 |
| Manual mint (`mint`, CLI-only) | `crates/orbit-core/src/auto_tasks/crud.rs` | ORB-10439 |
| Deterministic action | `crates/orbit-core/src/runtime/v2_host/dispatch.rs` | ORB-10149 |
| Seeded assets | `crates/orbit-core/assets/{activities,jobs,routines}/…` | ORB-10149 |

## Definitions shipped in this repo

- `qa-sweep` — hands-on validation of recent changes (ORB-10148).
- `artifact-deprecation-review` — report-only weekly review that lists stale
  learning candidates (usage rollups + anchor health) and stale artifact-id
  comment references (`L-`/`ADR-`/`ORB-`/`F` ids swept from source comments
  and resolved against their registries) via `execution_summary`; never
  mutates learnings, ADRs, tasks, friction records, or comments (ORB-10318,
  ORB-10348, [project-learnings §7.6](../project-learnings/2_design.md#76-recurring-deprecation-review-auto-task)).

## Task References

- ORB-10149 — Auto-task primitive.
- ORB-10148 — qa-sweep V1 (first definition; depends on this).
- ORB-10318 — learning-deprecation-review definition (report-only stale-learning review; superseded by ORB-10348).
- ORB-10348 — Generalized the definition into artifact-deprecation-review, adding the comment-reference sweep.
- ORB-10439 — on-demand manual mint (renamed to `orbit auto-task mint <name>` by ORB-10446).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
