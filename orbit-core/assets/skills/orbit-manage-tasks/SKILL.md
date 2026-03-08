---
name: orbit-manage-tasks
description: Must use this skill when updating, searching, or archiving orbit tasks. Do not use this skill to "create" task, use `orbit-create-task` skill instead.
---

# Orbit Manage Tasks

## Purpose

Provide a deterministic, auditable workflow to update, search, and archive Orbit tasks via the `orbit task` CLI, with explicit ID resolution and post-mutation verification.

## Scope

In scope:
- Update: `orbit task update`
- Search: `orbit task search`
- Archive: `orbit task archive`
- Approve: `orbit task approve`

Supporting commands:
- `orbit task show <id>`
- `orbit task list`

Out of scope unless explicitly requested:
- `orbit task delete`
- `orbit task unarchive`

## Task Lifecycle

Tasks follow a linear lifecycle:

```
proposed → backlog → in_progress → review → done
```

Any status can transition to `blocked`. If you have a task at hand that is in `in_progress`, and blocked from execution, transition it to `blocked`. 


## Operating Rules

- Use `orbit task` commands only. Do not edit backing files directly.
- Never invent task IDs. Resolve IDs from command output or search/list results.
- Use explicit flags for each requested change.
- After update/archive, verify with `orbit task show <id>`.
- Prefer `--json` for machine-readable output in automation/debug flows.
- Avoid destructive operations unless the user explicitly asks.
- If any attribution field is missing on an existing task, backfill via `orbit task update`.
- File-backed tasks persist as task bundles at `{{ORBIT_ROOT}}/tasks/<status>/<task_id>/`.
- Use `execution-summary.md` for the canonical task execution summary. Store any additional task-owned markdown or reports under `artifacts/`.


## Command Reference

### Update

```bash
orbit task update <id> \
  --execution-summary "<multi-line markdown content>" \
  --assigned-to "<identity_display_name_or_model_name>" \
  --status <proposed|backlog|in-progress|review|done|blocked> \
  --branch "<branch_name>" \
  --pr-number "<pr_number>"
```

### Search

```bash
orbit task search "<query>" --json
```

### Archive

```bash
orbit task archive <id>
```

### Approve

```bash
orbit task approve <id> --by "<approver>" --note "<note>"
```

## Standard Workflows

1. Create Task 
2. Update Task
3. Search Tasks
4. Archive Task

## Response Contract

After executing commands, respond with:
- Action performed (`created`, `updated`, `completed`, `blocked`, `approved`)
- Task ID(s)
- Important fields changed or confirmed
- Any failure with concrete next-step remediation

Keep responses concise, operational, and user-safe.
