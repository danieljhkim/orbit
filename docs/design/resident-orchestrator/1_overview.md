---
title: Resident Orchestrator — Overview
owner: grok
last_updated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: overview
type: design
summary: V1 is a drain job plus a workspace sequencer — epic_pipeline drains one epic; workspace_auto_pipeline drains loose leaves first then starts exactly one epic. The fire clock lives outside Orbit.
tags: [resident-orchestrator, epic, jobs, mcp]
paths: [".orbit/resources/jobs/**", "crates/orbit-core/assets/jobs/**", "crates/orbit-core/assets/activities/**"]
related_features: [resident-orchestrator, activity-job]
related_artifacts: [ORB-10775, ORB-10776, ORB-10779, ORB-10788]
---

# Resident Orchestrator — Overview

V1 is two Orbit jobs, not one command.

`epic_pipeline` is the drain: a **deterministic scan** looks at the workspace for (1) tasks
in `proposed`, `backlog`, or `blocked`, or (2) failed job-runs. If the scan is empty, the
job succeeds as a no-op. If anything is present, the job invokes a **long-running
orchestrator activity**. That agent **does not edit the repository**. It creates and ships
tasks, inspects runs, and writes a workspace **session log**. The job loops scan →
orchestrate until a later scan is empty or a bounded ceiling.

`workspace_auto_pipeline` is the sequencer ([ORB-10788], [Workspace auto is a sequencer, not a leaf ship](./4_decisions.md#workspace-auto-is-a-sequencer-not-a-leaf-ship)). One tick drains
**loose leaves** through existing ship, then — only when none remain — starts **exactly
one** `epic_pipeline`. An in-progress epic holds other auto-ship. `orbit run ship` stays
the leaf implementer; `orbit run auto` is the logistics verb.

The **clock** that fires either job is not an Orbit routine. A cron (or a human, or a
front-door session) on a separate knowledgebase checkout, with Orbit MCP wired in, calls
`orbit run auto` or `orbit run job epic_pipeline`. Selection still lives there.

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

**Orchestrator activity.** `epic_orchestrator`: one `backend: cli` agent_loop. It
**does not change code**. Allowlist is Orbit task + workflow/run + search +
`orbit.session_log.*` — no git write, no worktree edit, no `agent_implement`. It
shrinks the scan set by creating tasks, shipping explicit ids, and resuming or
cancelling failed runs. It must not merge PRs reserved for Daniel.

**Session log.** Workspace-scoped append-only notebook (`orbit.session_log`). Entry
kinds: `status`, `note`, `check_later`. Unresolved `check_later` rows are a scan
wake reason, so "look at this next time" actually wakes the next fire. This is the
memory between invokes; conversation resume stays out of v1.

**Epic job.** `epic_pipeline`: loop { scan; break if empty; invoke orchestrator } until
empty, iteration cap, or wall clock. A leftover scan after the cap fails the job
fail-closed; success is never inferred from agent prose.

**External clock.** Cron / knowledgebase supervisor / front door. Not a seeded Orbit
routine in v1.

**Epic tag.** Supervisor delegation for a root body of work ([Epic tag is a supervisor delegation signal, not the job predicate](./4_decisions.md#epic-tag-is-a-supervisor-delegation-signal-not-the-job-predicate)). It is **not**
`epic_pipeline`'s pickup key. It **is** the leaf-ship exclusion key ([Workspace auto is a sequencer, not a leaf ship](./4_decisions.md#workspace-auto-is-a-sequencer-not-a-leaf-ship)): auto
`list_backlog` skips the root and its descendants; explicit ship of the root is refused.

**Workspace auto.** `workspace_auto_pipeline` / `orbit run auto`: one logistics tick.
Loose leaves first; then one epic; hold while an epic is `in-progress`. Not a new
implementer and not a seeded routine.

## 3. At a Glance

| Concern | Where | Task |
|---------|-------|------|
| This split | this folder | [ORB-10776] |
| Epic tag = supervisor delegation signal | [Epic tag is a supervisor delegation signal, not the job predicate](./4_decisions.md#epic-tag-is-a-supervisor-delegation-signal-not-the-job-predicate) | [ORB-10776] |
| Clock and supervisor stay outside Orbit | [The supervisor clock is not an Orbit primitive](./4_decisions.md#the-supervisor-clock-is-not-an-orbit-primitive) | [ORB-10776] |
| `orbit.session_log` (notes / check-later / status) | workspace session-log store + tools | [ORB-10784] |
| `scan_unresolved_work` + `epic_orchestrator` + `epic_pipeline` | catalog | [ORB-10779] |
| Sequencer is not leaf ship | [Workspace auto is a sequencer, not a leaf ship](./4_decisions.md#workspace-auto-is-a-sequencer-not-a-leaf-ship) | [ORB-10788] |
| `workspace_auto_pipeline` + `orbit run auto` | catalog + CLI | [ORB-10788] |
| Child delivery while draining | existing `task_gate_pipeline` / `task_pr_pipeline` | Existing |
| HTTP epic retirement | removed assets | [ORB-10332] |

## Task References

- **[ORB-10332]** — Remove the unused HTTP epic pipeline.
- **[ORB-10775]** — Epic: drain job in Orbit; supervisor clock stays external.
- **[ORB-10776]** — Accept this contract; [Epic tag is a supervisor delegation signal, not the job predicate](./4_decisions.md#epic-tag-is-a-supervisor-delegation-signal-not-the-job-predicate) and [The supervisor clock is not an Orbit primitive](./4_decisions.md#the-supervisor-clock-is-not-an-orbit-primitive).
- **[ORB-10779]** — Ship the scan, the orchestrator activity, and `epic_pipeline`.
- **[ORB-10784]** — `orbit.session_log` (status / note / check_later).
- **[ORB-10788]** — Sequencer job, leaf-ship exclusion, `orbit run auto`.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
