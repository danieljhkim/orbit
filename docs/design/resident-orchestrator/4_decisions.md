---
title: Resident Orchestrator — Decisions
owner: grok
last_updated: 2026-08-14
last_validated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: decisions
type: design
summary: Decision log for the drain job, the external clock, and the split between leaf ship and workspace auto.
tags: [resident-orchestrator, epic, jobs]
paths: [".orbit/resources/jobs/**", "crates/orbit-core/assets/jobs/**"]
related_features: [resident-orchestrator, activity-job]
related_artifacts: [ORB-10332, ORB-10775, ORB-10776, ORB-10779, ORB-10788]
---

# Resident Orchestrator — Decisions

This document preserves the feature's non-obvious decisions and their reasoning.

---

## Epic tag is a supervisor delegation signal, not the job predicate

**Recorded:** 2026-08 · [ORB-10776]

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
- Leaf-ship exclusion uses the same tag ([Workspace auto is a sequencer, not a leaf ship](#workspace-auto-is-a-sequencer-not-a-leaf-ship)). That does not change this ADR's
  rule: `epic_pipeline` still must not require the tag to drain.

## The supervisor clock is not an Orbit primitive

**Recorded:** 2026-08 · [ORB-10776]

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

## Session log is the orchestrator's memory, not a CLI session

**Recorded:** 2026-08 · [ORB-10776]

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

## Drain scan excludes `epic_pipeline` runs

**Recorded:** 2026-08 · [ORB-10779]
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

## Workspace auto is a sequencer, not a leaf ship

**Recorded:** 2026-08 · [ORB-10788]
`orbit run auto`

### Context

Auto `orbit run ship` admitted an `epic`-tagged root (ORB-10775) as a leaf. The
implement pipeline found nothing to commit and parked the tracker in `blocked`.
Folding "start `epic_pipeline`" into `task_auto_pipeline` was the other alternative:
same verb, different child job, different success definition, exclusive
concurrency. That would make empty-backlog and `pipeline_success_guard` lie, and
would race the orchestrator on epic children.

### Decision

Keep `orbit run ship` a leaf implementer. Auto `list_backlog` skips any task that
is `tag: epic` or has such an ancestor (`epic_root` / `epic_child`). Explicit ship
of an epic root is refused before worktree setup; explicit ship of an epic child
stays allowed (the orchestrator path). Logistics live in a new job,
`workspace_auto_pipeline`, invoked as `orbit run auto`: drain loose leaves first;
if an epic root is `in-progress`, hold; else start exactly one backlog epic via
`epic_pipeline`. Do not seed a routine ([The supervisor clock is not an Orbit primitive](#the-supervisor-clock-is-not-an-orbit-primitive) still holds). Do not scope
`scan_unresolved_work` to one epic in this change.

### Consequences

- Two auto verbs. Muscle memory `orbit run ship` no longer starts an epic; operators
  who want logistics use `orbit run auto`.
- An in-progress epic blocks all auto-ship, including late-arriving loose chores.
- Cost: a third catalog job and a new CLI verb. Untagged backlog can still race if
  someone fires both `orbit run ship` and `epic_pipeline` on the same workspace.

## Task References

- **[ORB-10332]** — Remove the unused HTTP epic pipeline.
- **[ORB-10775]** — v1 implementation epic.
- **[ORB-10776]** — Record this split.
- **[ORB-10779]** — Ship the scan, the orchestrator, and `epic_pipeline`.
- **[ORB-10788]** — Sequencer and leaf-ship exclusion.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
