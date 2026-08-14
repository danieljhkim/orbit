---
title: Resident Orchestrator — Decisions
owner: grok
last_updated: 2026-08-14
last_validated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: decisions
type: design
summary: ADR log for workspace-addressed epic delegation and CLI-backed resident orchestration.
tags: [resident-orchestrator, epic, routines, cli]
paths: [".orbit/resources/activities/**", ".orbit/resources/jobs/**", ".orbit/routines/**"]
related_features: [resident-orchestrator, activity-job, routines]
related_artifacts: [ORB-10332, ORB-10775, ORB-10776, ADR-0361]
---

# Resident Orchestrator — Decisions

Decision record for resident-orchestrator, in ascending number order. This file is the
authoritative body — there is no ADR store behind it. Numbering is repo-local:
take the next unused number with `grep -rho 'ADR-[0-9]\{4\}' docs/ | sort -u | tail -1`.

An entry is admitted through exactly one of two doors: it explains a specific
code site that would otherwise look wrong (Door 1, carries `code_anchors:`), or
it states a standing rule that decides future tradeoffs (Door 2, carries
`scope:`). Everything else is design prose and belongs in `2_design.md`. See
[CONVENTIONS.md §4](../CONVENTIONS.md#4-adrs-strict).

## ADR-0361 — Epic tag is the sole resident pickup selector

**Status:** Accepted · 2026-08 · [ORB-10776]
**Scope:** resident epic selection and pickup (`select_resident_epic` and any successor)

### Context

The former `task_epic_pipeline` treated a root `TaskType::Feature` as an epic. That pipeline
was removed as unused in [ORB-10332]. A resident still needs an explicit delegation signal
that does not overload the broad `feature` type, and that future pickup code will not
reintroduce a second selector "just this once."

### Decision

Resident orchestration selects only root tasks tagged `epic`. The tag is a pickup
boundary, not a new task type and not a change to `TaskType`. Child work is recognized
by `parent_id`. `proposed` epics are never selected in v1.

### Consequences

- Creating a root `epic` in a workspace is the act of delegation.
- A later impulse to key pickup on `type: feature`, assignee, or folder path is out of
  contract unless this ADR is superseded.
- Cost: an untagged large feature is invisible to the resident even if a human thinks of
  it as an epic; pickup cannot be inferred from type or title.

## Task References

- **[ORB-10332]** — Remove the unused HTTP epic pipeline (`task_epic_pipeline`, `epic_orchestrator`).
- **[ORB-10775]** — v1 implementation epic.
- **[ORB-10776]** — Accept this folder and record ADR-0361.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
