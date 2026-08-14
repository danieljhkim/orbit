---
title: Resident Orchestrator — Design
owner: grok
last_updated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: design
type: design
summary: epic_pipeline loops a deterministic unresolved-work scan and a full-MCP orchestrator activity until the scan is empty. The fire clock is external.
tags: [resident-orchestrator, epic, jobs, mcp]
paths: [".orbit/resources/jobs/**", "crates/orbit-core/assets/jobs/**", "crates/orbit-core/assets/activities/**"]
related_features: [resident-orchestrator, activity-job]
related_artifacts: [ORB-10332, ORB-10775, ORB-10776, ORB-10779, ADR-0361, ADR-0362]
---

# Resident Orchestrator — Design

This is the v1 contract. It specifies the scan, the orchestrator activity, the job loop,
and what Orbit deliberately does not own. It does not add a resident server, an Orbit
routine, conversation resume, or a comment-typed decision protocol.

## 1. Unresolved-work scan

`scan_unresolved_work` is a deterministic activity. It reads the source workspace only.

It includes:

- every task with status `proposed`, `backlog`, or `blocked`;
- every job-run whose state is `failed` or `timeout`.

It excludes:

- `in-progress` tasks (a run is already live);
- `review` and `done` / `rejected` / `archived` / `someday` tasks;
- `cancelled` job-runs (operator-intentional);
- runs still `pending` / `running` / `retrying`.

Output is a structured object: `task_ids`, `run_ids`, counts, and a boolean `empty`.
No task or run is mutated. An empty scan is success, not an error.

The scan is the only admission function. The job does not select by `epic` tag, assignee,
or priority. Those filters belong to the external supervisor if it wants them (it can
decline to fire the job).

## 2. Orchestrator activity

`epic_orchestrator` is an `agent_loop` / `backend: cli` activity.

- **Tools:** the full Orbit MCP/CLI tool catalog for that workspace (task lifecycle,
  workflow run inspect/resume/ship, search). It is not a leaf `agent_implement` allowlist
  and it is not a new tool surface.
- **Mandate:** shrink the scan set. Typical moves: promote or reject `proposed`, re-dispatch
  or re-block with a precise reason, create children and `workflow_ship` explicit ids,
  `workflow_run_resume` a failed run whose cause is fixed, cancel a run that should stay
  dead and move the coupled task out of `blocked`.
- **Must not:** merge PRs reserved for Daniel; edit a second workspace; treat silence as
  new product authority.
- **Bound:** a wall-clock timeout on the activity (suggested 2 hours). The *job* is what
  retries the scan after the process exits. The agent is instructed to drain, but the
  scan after return is authoritative.

Conversation resume across fires is out of v1. Each invoke starts fresh from Orbit state.

## 3. `epic_pipeline` job

```text
loop
  scan = scan_unresolved_work
  break if scan.empty
  invoke epic_orchestrator
until empty or max_iterations or job wall clock
if scan not empty: fail
else: succeed
```

- Empty first scan: success, zero orchestrator invokes.
- The loop is how "work until resolved" is enforced. Agent prose cannot mark the job
  successful while `proposed`/`backlog`/`blocked` or failed/timeout runs remain.
- `max_iterations` and the job timeout are fail-closed ceilings, not "close enough."
- `overlap: forbid` at whatever *external* clock fires the job; the job itself should
  set `max_active_runs: 1` so two overlapping fires do not double-orchestrate.

Input is workspace-scoped (the runtime workspace). No `epic_id` is required. A supervisor
that wants to drain one epic only does so by not leaving other tasks in the scanned
statuses, or by running in a workspace that only holds that work.

## 4. External clock (not an Orbit routine)

v1 does not seed `resident-epic-orbit` or any other routine. A knowledgebase checkout
(or the front-door orchestrator) fires:

```text
orbit run job epic_pipeline
```

via MCP or CLI, on a cron it owns. That process may also create `epic`-tagged roots,
author children, and answer humans. None of that is a catalog asset in this repo.

## 5. Authority and completion

Daniel's merge authority is unchanged. `review` is not a wake reason: a task waiting on
human merge must not keep the drain loop alive by itself. If the only leftovers are
`review` + healthy runs, the scan is empty and the job succeeds.

A failed child pipeline that flipped its task to `blocked` *is* a wake reason (status
`blocked` and/or run `failed`). The orchestrator is expected to diagnose and either
resume, recast the task, or leave a precise block reason and — if it cannot progress —
exit so the next scan still sees the item and the job fails at the ceiling rather than
lying.

## 6. Concerns & Honest Limitations

- **Workspace-wide drain.** One `blocked` chore anywhere wakes the orchestrator for the
  whole workspace. Supervisors that want isolation should use separate workspaces or
  keep unrelated work out of `proposed`/`backlog`/`blocked`.
- **`proposed` is in the scan.** That is a deliberate reversal of the first draft (which
  refused proposed pickup). The orchestrator may triage; it still must not invent
  approval policy.
- **No conversation continuity.** Every fire re-reads Orbit. Cheap if the scan is empty;
  expensive if the cron is aggressive and the set is large.
- **Full MCP is trusted.** This activity is a workspace-admin loop, not a sandboxed leaf.
  The external clock is part of the security boundary.
- **Ceilings can fail a job with work left.** That is correct. The next cron fire tries
  again. Do not weaken the post-loop scan.

## Task References

- **[ORB-10332]** — Removed HTTP epic pipeline.
- **[ORB-10775]** — Implementation epic.
- **[ORB-10776]** — This contract and ADRs.
- **[ORB-10779]** — Scan, orchestrator activity, `epic_pipeline`.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
