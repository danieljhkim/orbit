---
name: orbit-task-pilot
description: Read-only preflight over a bounded partition of Orbit tasks — proposes canonical context_files, crew and complexity, dependencies, duplicate or already-landed evidence, and ADR conflicts, without promoting, dispatching, implementing, or mutating anything.
---

# Orbit Task Pilot

## Contract

Accept an explicit target workspace, target branch, and a partition of one to five Orbit task IDs. Inspect those tasks and return proposals. Do not apply them.

For each task:

1. Read the complete record and preserve the current selector list exactly as `context_files_before`.
2. Inspect the target branch, modification and deletion targets, related tasks, and relevant accepted ADRs using read-only tools.
3. Return `context_files_after` using only canonical `file:`, `dir:`, or `symbol:` selectors whose anchors exist inside the target workspace. Modification and deletion targets only — never read-for-context files.
4. Use disposition `selectors` for a non-empty proposal. An empty proposal is valid only with `verified_no_diff` or `host_operational`, plus concrete evidence.
5. Report recommendations for crew, complexity, real blockers, duplicates, already-landed work, utility, and public surface — without applying them, and without silently picking an architecture alternative.

## Hard bounds

Never write: no editing, creating, moving, or deleting repository files; no task updates or lifecycle transitions; no promotion, dispatch, implementation, pipeline invocation, commit, push, merge, PR, or approval.

Never expand the partition beyond the supplied IDs, except to cite a concrete dependency, duplicate, or accepted ADR.

Task-pilot is an operational role, not a crew, actor identity, or scheduler.

## Result

Return one partition object with the exact `partition_index` and `task_ids`, one assessment per supplied ID, and a short summary. Each assessment carries:

- `task_id`
- exact `context_files_before` and proposed `context_files_after`
- `disposition`, plus `evidence` when the proposal is empty
- `recommended_crew` and `recommended_complexity`
- `blocked_by`, `duplicate_of`, `already_landed`
- `adr_conflicts`, `utility_warnings`, `surface_warnings`

Fail the whole partition if any supplied task cannot be assessed. Never omit a task, and never summarize a failed partition as successful.
