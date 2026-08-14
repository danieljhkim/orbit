---
title: Resident Orchestrator — Decisions
owner: grok
last_updated: 2026-08-14
last_validated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: decisions
type: design
summary: ADR log for the drain job and the rule that the supervisor clock is not an Orbit primitive.
tags: [resident-orchestrator, epic, jobs]
paths: [".orbit/resources/jobs/**", "crates/orbit-core/assets/jobs/**"]
related_features: [resident-orchestrator, activity-job]
related_artifacts: [ORB-10332, ORB-10775, ORB-10776, ORB-10779, ADR-0361, ADR-0362, ADR-0363, ADR-0364]
---

# Resident Orchestrator — Decisions

Decision record for resident-orchestrator. This file is the authoritative body.

## ADR-0361 — Epic tag is a supervisor delegation signal, not the job predicate

**Status:** Accepted · 2026-08 · [ORB-10776]
**Scope:** how a body of work is marked for a supervisor; not `epic_pipeline` admission

### Context

The removed `task_epic_pipeline` treated `TaskType::Feature` as an epic. A later draft made
the `epic` tag the *job's* pickup selector. The v1 job instead wakes on
`proposed`/`backlog`/`blocked` plus failed/timeout runs, or it would miss ordinary chores
and failed pipelines that are not tagged.

### Decision

`epic` on a root task means "a supervisor owns this outcome." Catalog code must not require
the tag to drain work. Adding a second pickup key (`type: feature`, assignee, folder) for
the *job* is out of contract unless this ADR is superseded.

### Consequences

- Supervisors can still create `epic` roots and children as they do today (ORB-10775).
- Cost: a workspace with leftover `backlog` chores will wake the drain job even when no
  epic exists; isolation is a workspace-layout problem, not a tag filter.

## ADR-0362 — The supervisor clock is not an Orbit primitive

**Status:** Accepted · 2026-08 · [ORB-10776]
**Scope:** routines, activities, and jobs that would schedule or select work for `epic_pipeline`

### Context

The first draft specified `resident_orchestrator`, `select_resident_epic`, a JSON comment
protocol, conversation resume, and a seeded `resident-epic-orbit` routine. That rebuilds
an orchestrator inside Orbit next to Cowork / Grok / a knowledgebase cron that already
speak MCP.

### Decision

Orbit v1 ships only `scan_unresolved_work`, `epic_orchestrator`, and `epic_pipeline`.
Do not add an Orbit routine, selector activity, session-resume requirement, or
comment-typed mailbox for this feature. The fire clock lives in a knowledgebase (or
front-door) process that calls `orbit run job epic_pipeline`.

### Consequences

- A future "make it a routine" PR needs to supersede this ADR, not sneak a YAML into
  `.orbit/routines/`.
- Cost: no first-class resident health in `orbit routine list`; operators debug the
  external cron and the job-run log instead.

## ADR-0363 — Session log is the orchestrator's memory, not a CLI session

**Status:** Accepted · 2026-08 · [ORB-10776]
**Scope:** how `epic_orchestrator` remembers work across invokes; scan wake reasons

### Context

Each drain fire is a new CLI process. Conversation resume is out of v1. The orchestrator
still needs to leave itself status, notes, and "check this later," and to see new notes
on the next fire. Task comments are a human thread. A standing `backlog` log task would
wake the scan forever. A file in the knowledgebase cron repo is the wrong workspace.

### Decision

Give the workspace an append-only `orbit.session_log` with kinds `status`, `note`, and
`check_later`. Unresolved `check_later` entries are a `scan_unresolved_work` wake reason.
`status`/`note` are not. Resolve is the only mutation besides append. The orchestrator
does not edit repository files; code changes are child tasks it creates and ships.

### Consequences

- Next fire starts with `session_log.list` + the task/run scan, not a provider session id.
- Cost: another noun and three tools. Reminders the orchestrator forgets to `resolve`
  will keep waking the drain until someone does.

## ADR-0364 — Drain scan excludes `epic_pipeline` runs

**Status:** Accepted · 2026-08 · [ORB-10779]
**Code anchors:** `crates/orbit-core/src/runtime/v2_host/scan_unresolved.rs::scan_unresolved_work`

### Context

A leftover scan after `max_iterations` fails `epic_pipeline` closed. The next
external fire must see the leftover *tasks* and *child* failed/timeout runs, not
the drain job's own failed row. Including `epic_pipeline` itself would make every
ceiling failure a permanent wake reason and invite the orchestrator to resume
the drain from inside the drain.

### Decision

`scan_unresolved_work` omits job-runs whose `job_id` is `epic_pipeline`. Child
pipeline failures remain wake reasons. The supervisor clock starts a *new*
`epic_pipeline` run; it does not resume the previous drain via the scan set.

### Consequences

- A fail-closed drain can fire again on the next cron tick without first
  cancelling or resolving its own previous run.
- Cost: an operator cannot use the scan to discover a wedged `epic_pipeline`
  run; they use `orbit run history` / `orbit.workflow.run.list` instead.

## Task References

- **[ORB-10332]** — Remove the unused HTTP epic pipeline.
- **[ORB-10775]** — v1 implementation epic.
- **[ORB-10776]** — Record this split.
- **[ORB-10779]** — Ship the scan, the orchestrator, and `epic_pipeline`.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
