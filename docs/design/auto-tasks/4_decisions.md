---
title: Auto-tasks — Decisions
owner: claude
last_updated: 2026-07-12
status: Accepted
feature: auto-tasks
doc_role: decisions
type: design
summary: ADR log for the auto-task primitive.
tags: [auto-tasks]
paths: ["crates/orbit-core/src/auto_tasks/**"]
related_features: [auto-tasks]
related_artifacts: [ADR-0218, ADR-0219]
---

# Auto-tasks — Decisions

ADR log for auto-tasks. Entries are append-only and ordered by ascending global
ID. The store (`orbit.adr.show ADR-0218`) owns ID, status, owner, and links;
this file is the long-form narrative keyed on that same ID.

## ADR-0218 — Auto-task primitive: file-backed recurring task templates + one generic scheduler routine

**Status:** Accepted · 2026-07 · [ORB-10149]

**Context.** Every periodic need in orbit was bespoke code, so the marginal cost
of a new recurring chore was a code change, review, and release.

**Decision.** Introduce auto-tasks: a git-versioned YAML record
(`.orbit/auto_tasks/<name>.yaml`, modeled on the file-backed routine convention,
not the SQLite-indexed learning/ADR one) with a cron/interval `schedule`, an
`enabled` toggle, a task `template`, and a `dedupe` policy. One generic
deterministic activity (`run_auto_task_scheduler`, wrapped in a job, fired by a
seeded routine) mints tasks from the due definitions — the only scheduler
surface. Per-definition last-fired state is host-local under `.orbit/state/`.
Catch-up always collapses; `skip_if_open` dedupe keys on the `auto-task:<name>`
provenance tag. Provider-neutral per ADR-0217 — no turn knobs in the schema.
Full CRUD via CLI and MCP; disabling is a toggle, not a delete.

**Consequences.**

- Periodic work becomes data: a new chore is a definition, not orbit code.
  qa-sweep V1 (ORB-10148) is the first definition.
- Routine fires are observable on `/api/routines` for free, and the minutely
  sweep stays cheap because each definition's cursor + catch-up collapse make
  the pass idempotent.
- Definitions are workspace-scoped (the scheduler processes the firing
  workspace's `auto_tasks/`), and the host-local cursor keeps the git-versioned
  definition churn-free.
- Cost: a second file-backed record convention now exists alongside the
  SQLite-indexed one; the two must be kept mentally distinct, and auto-task
  definitions are not full-text searchable via the learning/ADR index.

## ADR-0219 — No-diff-expected tasks bypass repository change gates

**Status:** Accepted · 2026-07 · [ORB-10148]

**Context.** QA validation creates durable findings through Orbit tasks and may
correctly leave the repository unchanged. The normal empty-diff gate must still
fail closed for implementation tasks.

**Decision.** Make `no-diff-expected` a first-class task tag. Commit and PR
handoff skip their empty-stage and zero-commits-ahead failures only for an
explicitly tagged task (or a bundle in which every task is tagged). Such tasks
still require a meaningful execution summary and follow the normal lifecycle;
Orbit creates neither an empty commit nor an empty PR.

**Consequences.**

- Side-effect-only validation tasks flow through normal orchestrator dispatch.
- Ordinary implementation tasks retain fail-closed empty-diff checks.
- The QA auto-task definition carries the exemption visibly in its template.
- Cost: a mistakenly tagged task can reach review without repository changes,
  so the tag is a privileged workflow exemption.

## Task References

- [ORB-10149] — Shipped the auto-task primitive (record, scheduler, CRUD, assets).
- [ORB-10148] — Added the QA definition and no-diff workflow exemption.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
