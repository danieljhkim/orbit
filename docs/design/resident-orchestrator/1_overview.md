---
title: Resident Orchestrator — Overview
owner: grok
last_updated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: overview
type: design
summary: V1 is one catalog job — scan unresolved work, then run a long-lived orchestrator with full Orbit MCP until the scan is empty. The clock that fires it lives outside Orbit.
tags: [resident-orchestrator, epic, jobs, mcp]
paths: [".orbit/resources/jobs/**", "crates/orbit-core/assets/jobs/**", "crates/orbit-core/assets/activities/**"]
related_features: [resident-orchestrator, activity-job]
related_artifacts: [ORB-10775, ORB-10776, ORB-10779, ADR-0361, ADR-0362]
---

# Resident Orchestrator — Overview

V1 is one Orbit job, `epic_pipeline`.

A **deterministic scan** looks at the workspace for (1) tasks in `proposed`, `backlog`, or
`blocked`, or (2) failed job-runs. If the scan is empty, the job succeeds as a no-op. If
anything is present, the job invokes a **long-running orchestrator activity** with the full
Orbit MCP tool surface. That agent works the set until a later scan is empty. The job loops
scan → orchestrate until drain or a bounded iteration/time ceiling.

The **clock** that fires this job is not an Orbit routine. A cron (or a human, or a front-door
session) on a separate knowledgebase checkout, with Orbit MCP wired in, calls
`orbit run job epic_pipeline`. Selection, decomposition, and "should we run now?" live there.

This supersedes the unused HTTP `task_epic_pipeline` ([ORB-10332]) and the first draft's
in-Orbit resident (session resume, JSON comment protocol, `select_resident_epic`, seeded
routine). Those are not v1.

## 1. Motivation

Leaf pipelines ship one task. A front-door orchestrator can already speak MCP. What Orbit
lacked is a single, callable "there is unfinished work — stay on it until it is gone"
primitive. Putting the supervisor *inside* Orbit (a 2-hour CLI resident, session resume,
decision-comment protocol, workspace routine) duplicates the orchestrators we already run.

The useful Orbit piece is the drain loop. The useful outside piece is the cron that decides
when to start one.

## 2. Core Concepts

**Unresolved work.** A workspace-local set: every task whose status is `proposed`,
`backlog`, or `blocked`, plus every job-run in `failed` or `timeout`. `in-progress` and
`review` are live or waiting on a human; the scan does not treat them as wake reasons.
`cancelled` runs are intentional and are not wake reasons.

**Scan.** Deterministic activity `scan_unresolved_work`. Read-only. Returns the set (ids +
counts). Empty set is a successful no-op input to the job.

**Orchestrator activity.** `epic_orchestrator`: one `backend: cli` agent_loop with the
full Orbit tool catalog (task, workflow/run, search — not a leaf implementer allowlist).
Its mandate is to shrink the scan set: triage `proposed`, unblock or re-dispatch
`blocked`, ship or decompose `backlog`, resume or otherwise close failed/timeout runs.
It must not merge PRs that policy reserves for Daniel.

**Epic job.** `epic_pipeline`: loop { scan; break if empty; invoke orchestrator } until
empty, iteration cap, or wall clock. A leftover scan after the cap fails the job
fail-closed; success is never inferred from agent prose.

**External clock.** Cron / knowledgebase supervisor / front door. Not a seeded Orbit
routine in v1.

**Epic tag.** Still the supervisor's delegation convention for a root body of work
(ADR-0361). The job predicate is status + failed runs, not the tag.

## 3. At a Glance

| Concern | Where | Task |
|---------|-------|------|
| This split | this folder | [ORB-10776] |
| Epic tag = supervisor delegation signal | ADR-0361 | [ORB-10776] |
| Clock and supervisor stay outside Orbit | ADR-0362 | [ORB-10776] |
| `scan_unresolved_work` + `epic_orchestrator` + `epic_pipeline` | catalog | [ORB-10779] |
| Child delivery while draining | existing `task_gate_pipeline` / `task_pr_pipeline` | Existing |
| HTTP epic retirement | removed assets | [ORB-10332] |

## Task References

- **[ORB-10332]** — Remove the unused HTTP epic pipeline.
- **[ORB-10775]** — Epic: drain job in Orbit; supervisor clock stays external.
- **[ORB-10776]** — Accept this contract; ADR-0361 and ADR-0362.
- **[ORB-10779]** — Ship the scan, the orchestrator activity, and `epic_pipeline`.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
