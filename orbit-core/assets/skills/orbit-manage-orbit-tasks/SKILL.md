---
name: orbit-manage-orbit-tasks
description: Manage tasks through deterministic CLI workflows for create, update, search, and close operations. Use this skill when the user asks to create, modify, find, or close orbit tasks.
---

# Manage Orbit Tasks

## Purpose

Provide a deterministic, auditable way to create, update, search, and close Orbit tasks using the `orbit task` CLI, with explicit ID resolution and verification after mutations.

## Scope

In scope:
- Create: `orbit task add`
- Update: `orbit task update`
- Search: `orbit task search`
- Approve: `orbit task approve`
- Close: `orbit task close`

Supporting commands:
- `orbit task show <id>`
- `orbit task list`

Out of scope unless explicitly requested:
- `orbit task delete`
- `orbit task reopen`

## Operating Rules

- Use `orbit task` commands only. Do not edit backing files directly.
- Never invent task IDs. Resolve IDs from command output or search/list results.
- Use explicit flags for each requested change.
- After create/update/close, verify with `orbit task show <id>`.
- Prefer `--json` for machine-readable output in automation/debug flows.
- Avoid destructive operations unless the user explicitly asks.

## Command Reference

### Create

```bash
orbit task add \
  --title "<title>" \
  --description "<description>" \
  --instructions "<instructions>" \
  --context "<comma,separated,context>" \
  --workspace "<absolute_or_relative_repo_path>" \
  --priority <low|medium|high|critical> \
  --type <task|feature|issue|other> \
  --owner "<owner>" \
  --parent "<parent_id>"
```

Minimum required:
- `--title`

### Update

```bash
orbit task update <id> \
  --title "<title>" \
  --description "<description>" \
  --instructions "<instructions>" \
  --context "<comma,separated,context>" \
  --workspace "<absolute_or_relative_repo_path>" \
  --status <todo|in-progress|done|blocked|cancelled> \
  --priority <low|medium|high|critical> \
  --type <task|feature|issue|other> \
  --owner "<owner>" \
  --parent "<parent_id>"
```

Field-clearing notes:
- Clear parent: `--parent ""`
- Clear context: `--context ""`
- Clear workspace: `--workspace ""`

### Search

```bash
orbit task search "<query>"
```

Machine-readable:

```bash
orbit task search "<query>" --json
```

### Close

```bash
orbit task close <id>
```

Verify close:

```bash
orbit task show <id>
```

### Approve

```bash
orbit task approve <id> --by "<approver>" --note "<optional note>"
```

## Standard Workflows

### 1) Create Task

1. Collect required fields from user request.
2. Run `orbit task add ...`.
3. Capture returned task ID.
4. Run `orbit task show <id>`.
5. Report ID and key fields (title, status, priority, owner).

### 2) Update Task

1. If no ID is provided, run `orbit task search "<query>" --json`.
2. Resolve the correct ID.
3. Run `orbit task update <id> ...` with only requested changes.
4. Run `orbit task show <id>` and report final state.

### 3) Search Tasks

1. Run `orbit task search "<query>"` (or `--json`).
2. Return matches with ID, title, status, priority, owner.
3. If no matches, state that and offer creation next.

### 4) Close Task

1. Resolve ID directly or via search.
2. Run `orbit task close <id>`.
3. Run `orbit task show <id>` to confirm.
4. Report closed status.

## Response Contract

After executing commands, respond with:
- Action performed (`created`, `updated`, `found`, `closed`)
- Task ID(s)
- Important fields changed or confirmed
- Any failure with concrete next-step remediation

Keep responses concise, operational, and user-safe.
