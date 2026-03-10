---
name: orbit-manage-tasks
description: Must use this skill when updating or searching orbit tasks. Do not use this skill to "create" task, use `orbit-create-task` skill instead.
---

# Orbit Manage Tasks

## Purpose

Provide a deterministic, auditable workflow to update, search, approve, show, and archive Orbit tasks via the `orbit task` CLI.

## Scope

In scope:
- `orbit task update`
- `orbit task search`
- `orbit task show <id>`
- `orbit task list`


## Task Lifecycle

```text
proposed -> backlog -> in-progress -> review -> done
```

Any status may move to `blocked` when execution cannot safely continue.

## Operating Rules

- Use `orbit task` commands only; do not edit task files directly.
- Never invent task IDs; resolve them from search/list/show output.
- Use explicit flags for every requested change.
- Verify mutations with `orbit task show <id>`.
- Prefer `--json` for machine-readable flows.
- Avoid destructive operations unless explicitly requested.
- Backfill missing attribution fields when needed.
- Task bundles live at `{{ORBIT_ROOT}}/tasks/<status>/<task_id>/`.
- CLI-facing status spelling is `in-progress`; the on-disk task bundle directory remains `in_progress`.
- Use `execution-summary.md` for canonical execution summaries; store other task-owned artifacts under `artifacts/`.

## Command Reference

```bash
orbit task update <id> \
  --execution-summary "<multi-line markdown content>" \
  --assigned-to "<identity_display_name>" \
  --status <proposed|backlog|in-progress|review|done|blocked> \
  --branch "<branch_name>" \
  --pr-number "<pr_number>"
orbit task search "<query>" --json
orbit task show <id>
orbit task list
```

## Response Contract

After task commands, report:

- action performed (`updated`, `completed`, `blocked`, `found`)
- task ID(s)
- important fields changed or confirmed
- any failure and the next remediation step

Keep responses concise, operational, and user-safe.
