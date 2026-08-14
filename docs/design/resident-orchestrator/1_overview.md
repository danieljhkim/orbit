---
title: Resident Orchestrator — Overview
owner: grok
last_updated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: overview
type: design
summary: Workspace-addressed epic delegation to a resumable CLI orchestrator that decomposes, clarifies, and shepherds work without a resident server.
tags: [resident-orchestrator, epic, routines, cli, decision-gates]
paths: [".orbit/resources/activities/**", ".orbit/resources/jobs/**", ".orbit/routines/**", "crates/orbit-core/assets/**"]
related_features: [resident-orchestrator, activity-job, routines, agent-families]
related_artifacts: [ORB-10775, ORB-10776, ORB-10777, ORB-10778, ORB-10779, ORB-10780, ORB-10781, ORB-10782, ADR-0361]
---

# Resident Orchestrator — Overview

A **resident orchestrator** is a specialized agent bound to one Orbit workspace. The workspace is
its address, a root task tagged `epic` is its durable work order, and a workspace-local routine
wakes a bounded `backend: cli` ownership cycle. The resident decomposes the epic into normal child
tasks, ships explicit child task IDs through the existing delivery workflows, and shepherds the
whole tree to a verified terminal state. No per-agent server, inbox pump, or retained HTTP agent
session is required. Bounded CLI processes may resume an opaque provider conversation between
cycles, while Orbit remains the durable source of truth.

## 1. Motivation

The constellation currently has two useful but insufficient levels of delegation:

1. A front-door orchestrator can make cross-workspace decisions.
2. A one-shot runner can execute one scoped leaf mandate.

What is missing is durable, codebase-local ownership between those levels. A single front door
cannot retain deep context for several codebases while also following every task, workflow run,
review, merge conflict, and acceptance criterion to completion. Repeated generic runner calls do
not solve that problem: the front door remains the shepherd and must reconstruct the codebase's
state on every turn.

Orbit already provides the durable pieces needed for a smaller solution: workspace routing,
tagged tasks, parent/child task relationships, dependencies, CLI agent activities, jobs, routines,
and explicit-ID shipment workflows. The missing mechanism is a convention and a thin pickup cycle
that hand ownership of a high-level task to the workspace's specialized orchestrator.

The former `task_epic_pipeline` was not that mechanism. It assumed child tasks already existed,
used an HTTP agent loop with retained session state, and treated `review` as the end of automated
shipment. Orbit v1 supports CLI agent invocation; the resident must also own decomposition and the
post-review delivery loop. This design supersedes the HTTP epic path, which was removed as unused
in [ORB-10332], rather than porting it.

## 2. Core Concepts

**Resident workspace.** One Orbit workspace is the durable address for one specialized
orchestrator. For example, an Orbit systems agent receives work in the Orbit codebase workspace;
no separate agent queue is required.

**Epic assignment.** A root task with no `parent_id` and the tag `epic`. Creating that task in a
workspace is the act of delegation. Its description and acceptance criteria define the outcome;
the resident authors the execution plan after pickup.

**Resident activity.** A workspace-local `agent_loop` activity using `backend: cli`. It explicitly
binds the provider, model, and identity-loading instruction for that workspace's resident agent.
Different workspaces can use different resident agents without a new invocation service.

**Ownership cycle.** One bounded resident invocation. It resumes an active epic before selecting a
new one, advances as much of its task tree as current state permits, persists every decision in
Orbit, records a compatible provider conversation reference, and exits. A later routine fire may
resume that conversation for continuity, but must reconstruct the safe action from durable state.

**Decision gate.** A structured question recorded on the parent epic when consequential ambiguity
or missing authority makes further work disproportionately risky. The resident exits while the
question is pending; the next bounded cycle resumes with the matched supervisor or human answer.
V1 has no live mid-turn steering or managed interactive terminal.

**Child task.** A normal task created with `parent_id` pointing to the epic. Child tasks use the
ordinary lifecycle, dependency, crew, validation, review, and PR machinery; the `epic` tag does not
create a second execution model.

**Shepherding.** The resident's responsibility for the entire workspace-local delivery loop:
decomposition, explicit dispatch, run observation, failure recovery,
conflict/finding resolution, merge verification, child closure, and finally parent closure.

## 3. At a Glance

| Concern | File | Task |
|---------|------|------|
| Design acceptance and Grok feature-lead | this folder | [ORB-10776] |
| Epic selection and resume | proposed deterministic `select_resident_epic` activity | [ORB-10777] |
| Supervisor/human decision path | structured parent-task request and answer comments | [ORB-10778] |
| Resident CLI identity | workspace `.orbit/resources/activities/resident_orchestrator.yaml` | [ORB-10779] |
| Bounded ownership cycle | proposed `resident_epic_cycle` job | [ORB-10779] |
| Conversation continuity | Grok CLI session capture and resume adapter | [ORB-10780] |
| Scheduled pickup | workspace `.orbit/routines/resident-epic-orbit` (disabled until canary) | [ORB-10781] |
| `ws_orbit` grok canary | activity binding + routine enable after dry-run | [ORB-10782] |
| Child delivery | existing `task_gate_pipeline` / explicit-ID shipment workflows | Existing mechanism |
| HTTP epic retirement | former `task_epic_pipeline` and `epic_orchestrator` assets (removed in [ORB-10332]) | [ORB-10332] |

## Task References

- **[ORB-10332]** — Remove the unused HTTP epic pipeline assets (`task_epic_pipeline`, `epic_orchestrator`) this design supersedes.
- **[ORB-10775]** — Epic: resident orchestrator v1 (workspace-addressed CLI epic cycles).
- **[ORB-10776]** — Accept this folder and claim `owner: grok`.
- **[ORB-10777]** — Deterministic `select_resident_epic`.
- **[ORB-10778]** — Checkpoint and decision comment protocol.
- **[ORB-10779]** — `resident_orchestrator` activity and `resident_epic_cycle` job.
- **[ORB-10780]** — Grok CLI conversation capture/resume.
- **[ORB-10781]** — Disabled `resident-epic-orbit` routine.
- **[ORB-10782]** — `ws_orbit` grok canary.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
