---
title: Resident Orchestrator — Design
owner: grok
last_updated: 2026-08-14
status: Accepted
feature: resident-orchestrator
doc_role: design
type: design
summary: epic_pipeline loops a deterministic unresolved-work scan and a no-code-change orchestrator (create/ship/log) until the scan is empty. Session log is the memory between fires.
tags: [resident-orchestrator, epic, jobs, session-log]
paths: [".orbit/resources/jobs/**", "crates/orbit-core/assets/jobs/**", "crates/orbit-core/assets/activities/**"]
related_features: [resident-orchestrator, activity-job]
related_artifacts: [ORB-10332, ORB-10775, ORB-10776, ORB-10779, ADR-0361, ADR-0362, ADR-0363]
---

# Resident Orchestrator — Design

This is the v1 contract. It specifies the scan, the orchestrator activity, the job loop,
and what Orbit deliberately does not own. It does not add a resident server, an Orbit
routine, conversation resume, or a comment-typed decision protocol.

## 1. Unresolved-work scan

`scan_unresolved_work` is a deterministic activity. It reads the source workspace only.

It includes:

- every task with status `proposed`, `backlog`, or `blocked`;
- every job-run whose state is `failed` or `timeout`;
- every unresolved session-log entry with `kind: check_later` (ADR-0363).

It excludes:

- `in-progress` tasks (a run is already live);
- `review` and `done` / `rejected` / `archived` / `someday` tasks;
- `cancelled` job-runs (operator-intentional);
- runs still `pending` / `running` / `retrying`.

Output is a structured object: `task_ids`, `run_ids`, `check_later_ids`, counts, and a
boolean `empty`. No task, run, or log row is mutated. An empty scan is success, not an
error.

The scan is the only admission function. The job does not select by `epic` tag, assignee,
or priority. Those filters belong to the external supervisor if it wants them (it can
decline to fire the job).

## 2. Orchestrator activity

`epic_orchestrator` is an `agent_loop` / `backend: cli` activity.

- **Tools (allowlist):** `orbit.task.*` (add/update/show/list — not a license to edit
  files), workflow run inspect / resume / ship, `orbit.search`, `orbit.session_log.*`.
- **Not on the allowlist:** git write, worktree edit, `agent_implement`, `fs` writes
  into the repository, PR merge. A child pipeline makes the code change. If the
  orchestrator needs a patch, it files a task and ships it.
- **Mandate:** shrink the scan set. Typical moves: create children, `workflow_ship`
  explicit ids, promote or reject `proposed`, resume or cancel a failed run, append
  session-log status, file or resolve `check_later` notes.
- **Must not:** merge PRs reserved for Daniel; edit a second workspace; treat silence
  as new product authority; "just fix the file."
- **Bound:** a wall-clock timeout on the activity (suggested 2 hours). The *job*
  re-scans after the process exits. The agent is instructed to drain; the scan after
  return is authoritative.

Conversation resume across fires is out of v1. Each invoke starts fresh from Orbit
state plus `orbit.session_log.list`.

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

## 4. Session log

The orchestrator has no retained CLI conversation. It needs a notebook that survives a
fresh invoke: status of the last fire, notes to self, and "check this later."

That is `orbit.session_log`, a **workspace-scoped, append-only** store. It is not a
task, not a comment thread, and not a knowledgebase markdown file (the drain runs in
the *target* workspace; the cron repo is the wrong disk).

Each entry:

| Field | Rule |
|---|---|
| `id` | allocated, stable |
| `at` | timestamp |
| `kind` | `status` \| `note` \| `check_later` |
| `body` | markdown |
| `related_task_ids` / `related_run_ids` | optional |
| `resolved_at` | set only on `check_later` via `resolve` |

Tools:

- `orbit.session_log.append`
- `orbit.session_log.list` (filters: `kind`, `unresolved_only`, `since`)
- `orbit.session_log.resolve` (`check_later` id)

`status` and `note` never wake the scan. Unresolved `check_later` **does** — that is
how "remind me" works without session resume. Resolving a check-later is how the
orchestrator tells the next scan the reminder is done.

Rejected shapes: stuffing JSON into task comments (rejected with ORB-10778); a standing
`backlog` "log task" (it would wake the drain forever); a file in the knowledgebase
cron repo (wrong workspace).

Do not rewrite history. Append or resolve; never edit a body in place.

## 5. External clock (not an Orbit routine)

v1 does not seed `resident-epic-orbit` or any other routine. A knowledgebase checkout
(or the front-door orchestrator) fires:

```text
orbit run job epic_pipeline
```

via MCP or CLI, on a cron it owns. That process may also create `epic`-tagged roots,
author children, and answer humans. None of that is a catalog asset in this repo.

## 6. Authority and completion

Daniel's merge authority is unchanged. `review` is not a wake reason: a task waiting on
human merge must not keep the drain loop alive by itself. If the only leftovers are
`review` + healthy runs and no unresolved `check_later` notes, the scan is empty and
the job succeeds.

A failed child pipeline that flipped its task to `blocked` *is* a wake reason (status
`blocked` and/or run `failed`). The orchestrator is expected to diagnose and either
resume, recast the task, or leave a precise block reason and — if it cannot progress —
exit so the next scan still sees the item and the job fails at the ceiling rather than
lying.

## 7. Concerns & Honest Limitations

- **Workspace-wide drain.** One `blocked` chore anywhere wakes the orchestrator for the
  whole workspace. Supervisors that want isolation should use separate workspaces or
  keep unrelated work out of `proposed`/`backlog`/`blocked`.
- **`proposed` is in the scan.** That is a deliberate reversal of the first draft (which
  refused proposed pickup). The orchestrator may triage; it still must not invent
  approval policy.
- **No conversation continuity.** Every fire re-reads Orbit and the session log. Cheap
  if the scan is empty; expensive if the cron is aggressive and the set is large.
- **Allowlist is trusted but narrow.** The orchestrator can create and ship work; it
  cannot patch the tree. A wedged child that "only needs a one-line fix" stays wedged
  until a shipped child lands. That is the point.
- **Ceilings can fail a job with work left.** That is correct. The next cron fire tries
  again. Do not weaken the post-loop scan.

## Task References

- **[ORB-10332]** — Removed HTTP epic pipeline.
- **[ORB-10775]** — Implementation epic.
- **[ORB-10776]** — This contract and ADRs.
- **[ORB-10779]** — Scan, orchestrator activity, `epic_pipeline`.
- **[ORB-10784]** — `orbit.session_log`.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
