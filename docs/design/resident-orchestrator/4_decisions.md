---
title: Resident Orchestrator — Decisions
owner: codex
last_updated: 2026-07-18
status: Draft
feature: resident-orchestrator
doc_role: decisions
type: design
summary: ADR log for workspace-addressed epic delegation and CLI-backed resident orchestration.
tags: [resident-orchestrator, epic, routines, cli]
paths: [".orbit/resources/activities/**", ".orbit/resources/jobs/**", ".orbit/routines/**"]
related_features: [resident-orchestrator, activity-job, routines]
related_artifacts: []
---

# Resident Orchestrator — Decisions

ADR log for Resident Orchestrator. Entries are append-only and ordered by ascending global ID.
Allocate every `ADR-NNNN` through `orbit.adr.add` before adding its heading. The store remains the
source of truth for status, owner, related features, and related tasks.

This folder is still Draft and has no allocated ADR. The candidate choices described in
[2_design.md](./2_design.md)—workspace-addressed `epic` tasks, bounded CLI ownership cycles, and
replacement rather than conversion of the HTTP epic pipeline—remain proposals until the design is
accepted and implementation tasks are allocated.

## Candidate Decision: Epic Marker and Legacy Coexistence

**Proposal.** Resident orchestration selects root tasks by the `epic` tag rather than changing or
depending on `TaskType`. The legacy `task_epic_pipeline` continues to identify epics by
`TaskType::Feature` until it is retired.

**Coexistence rule.** The paths are disjoint by workspace capability during retirement stages 1–3.
Once a workspace enables its resident routine, legacy epic-pipeline admission must reject that
workspace; workspaces without the resident capability may continue to use the Feature-typed legacy
path. A task may carry both markers, but may never be claimed by both paths.

**Why this proposal.** A tag is an explicit delegation signal that does not overload the broad
`feature` type, while workspace-level exclusion provides a fail-closed migration boundary without
rewriting existing legacy tasks. This remains a candidate decision, not an allocated ADR.

## Task References

- None yet — implementation tasks will be allocated after this Draft is accepted.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
