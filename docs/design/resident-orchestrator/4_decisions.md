---
title: Resident Orchestrator — Decisions
owner: codex
last_updated: 2026-07-20
status: Draft
feature: resident-orchestrator
doc_role: decisions
type: design
summary: ADR log for workspace-addressed epic delegation and CLI-backed resident orchestration.
tags: [resident-orchestrator, epic, routines, cli]
paths: [".orbit/resources/activities/**", ".orbit/resources/jobs/**", ".orbit/routines/**"]
related_features: [resident-orchestrator, activity-job, routines]
related_artifacts: [ORB-10332]
---

# Resident Orchestrator — Decisions

ADR log for Resident Orchestrator. Entries are append-only and ordered by ascending global ID.
Allocate every `ADR-NNNN` through `orbit.adr.add` before adding its heading. The store remains the
source of truth for status, owner, related features, and related tasks.

This folder is still Draft and has no allocated ADR. The candidate choices described in
[2_design.md](./2_design.md)—workspace-addressed `epic` tasks, bounded CLI ownership cycles, and
replacement rather than conversion of the HTTP epic pipeline—remain proposals until the design is
accepted and implementation tasks are allocated. The HTTP epic pipeline this design supersedes was
itself removed as unused in [ORB-10332].

## Candidate Decision: Epic Marker and Legacy Coexistence

**Proposal.** Resident orchestration selects root tasks by the `epic` tag rather than changing or
depending on `TaskType`. The former `task_epic_pipeline` identified epics by `TaskType::Feature`;
[ORB-10332] removed that pipeline, so the `epic` tag is now the sole epic selector.

**Coexistence rule (obsolete).** The original proposal kept the two paths disjoint by workspace
capability during a staged retirement. With the legacy pipeline removed in [ORB-10332], there is no
second claimant to exclude, so this rule no longer applies.

**Why this proposal.** A tag is an explicit delegation signal that does not overload the broad
`feature` type. This remains a candidate decision, not an allocated ADR.

## Task References

- **[ORB-10332]** — Remove the unused HTTP epic pipeline (`task_epic_pipeline`, `epic_orchestrator`).
- Further implementation tasks will be allocated after this Draft is accepted.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
