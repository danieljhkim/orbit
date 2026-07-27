---
name: orbit-task-pilot
description: Read-only preflight for a bounded partition of Orbit tasks. Use when an orchestrator needs exact canonical context_files proposals, crew/complexity and dependency recommendations, duplicate or already-landed evidence, ADR conflicts, and utility/surface warnings without promotion, dispatch, implementation, or task mutation.
---

# Orbit Task Pilot

## Contract

Accept only an explicit target workspace, target branch, and a partition of one
to five Orbit task IDs. Inspect those tasks and return proposals; do not apply
them.

For each task:

1. Read the complete task record and preserve the current selector list exactly
   as `context_files_before`.
2. Inspect the target branch, modification/deletion targets, related tasks, and
   relevant accepted ADRs using read-only tools.
3. Return `context_files_after` using only canonical `file:`, `dir:`, or
   `symbol:` selectors whose anchors exist inside the target workspace. Include
   modification/deletion targets only, never read-for-context files.
4. Use disposition `selectors` for a non-empty proposal. An empty proposal is
   valid only with `verified_no_diff` or `host_operational` plus concrete
   evidence.
5. Report recommendations for crew, complexity, real blockers, duplicates,
   already-landed work, ADR conflicts, utility, and public surface. Do not apply
   those recommendations or silently select an architecture alternative.

## Hard bounds

- Never edit, create, move, or delete repository files.
- Never update an Orbit task or transition lifecycle state.
- Never promote, dispatch, implement, invoke a pipeline, commit, push, merge,
  open a PR, or approve work.
- Never expand the partition beyond the supplied IDs except to cite a concrete
  dependency, duplicate, or accepted ADR.
- Task-pilot is an operational role, not a provider, crew, actor identity,
  capability, scheduler, or task store.

## Result

Return one partition object containing the exact `partition_index` and
`task_ids`, one task assessment per supplied ID, and a short summary. Each
assessment contains:

- `task_id`
- exact `context_files_before` and proposed `context_files_after`
- `disposition` and, for an empty proposal, `evidence`
- `recommended_crew` and `recommended_complexity`
- `blocked_by`, `duplicate_of`, and `already_landed`
- `adr_conflicts`, `utility_warnings`, and `surface_warnings`

Fail the whole partition if any supplied task cannot be assessed. Never omit a
task or summarize a failed partition as successful.
