---
name: orbit-task-pilot
description: Read-only preflight over a bounded partition of Orbit tasks — proposes canonical context_files, crew and complexity, dependencies, duplicate or already-landed evidence, and conflicts with recorded decisions, without promoting, dispatching, implementing, or mutating anything.
---

# Orbit Task Pilot

## Contract

Accept an explicit target workspace, target branch, and a partition of one to five Orbit task IDs. Inspect those tasks and return proposals. Do not apply them.

For each task:

1. Read the complete record and preserve the current selector list exactly as `context_files_before`.
2. Inspect the target branch, modification and deletion targets, related tasks, and any decision entries governing the code in scope, using read-only tools. Decisions are titled sections in the feature's decision doc — reach them with `--kind doc`, not an ADR store.
3. Return `context_files_after` using only canonical `file:`, `dir:`, or `symbol:` selectors whose anchors exist inside the target workspace. Modification and deletion targets only — never read-for-context files.
4. Use disposition `selectors` for a non-empty proposal. An empty proposal is valid only with `verified_no_diff` or `host_operational`, plus concrete evidence.
5. Report recommendations for crew, complexity, real blockers, duplicates, already-landed work, utility, and public surface — without applying them, and without silently picking an architecture alternative.

## Hard bounds

Never write: no editing, creating, moving, or deleting repository files; no task updates or lifecycle transitions; no promotion, dispatch, implementation, pipeline invocation, commit, push, merge, PR, or approval.

Never update an Orbit task.

Never expand the partition beyond the supplied IDs, except to cite a concrete dependency, duplicate, or recorded decision.

Task-pilot is an operational role, not a crew, actor identity, or scheduler.

## Orchestrator handoff

The Luna worker described here is read-only. It must not update tasks or invoke
the pipeline. When ship traffic is high, the orchestrator should run
`orbit job show task_pilot_pipeline` and then `orbit run job task_pilot_pipeline`
so the apply step can persist validated selectors and reduce file-collision
risk. Do not fill `context_files` inline under traffic. The zero-input job mode
discovers eligible proposed/backlog tasks with empty selectors; explicit
`task_ids` audits exactly the named tasks. An enabled workspace routine may
already run the zero-input job on a schedule, but orchestrators can invoke an
extra run before reservation or conflict checks.

## Result

Return one partition object with the exact `partition_index` and `task_ids`, one assessment per supplied ID, and a short summary. Each assessment carries:

- `task_id`
- exact `context_files_before` and proposed `context_files_after`
- `disposition`, plus `evidence` when the proposal is empty
- `recommended_crew` and `recommended_complexity`
- `blocked_by`, `duplicate_of`, `already_landed`
- `adr_conflicts`, `utility_warnings`, `surface_warnings`

Fail the whole partition if any supplied task cannot be assessed. Never omit a task, and never summarize a failed partition as successful.
