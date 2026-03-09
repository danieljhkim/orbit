---
name: orbit-create-task
description: Use this when you need to create an Orbit task.
---

# Orbit Create Task

## Purpose

Create an Orbit task another engineer or agent can execute without guessing. Plans should be incremental and explicit about files, commands, risks, and verification.

## Workflow

1. Confirm objective, constraints, and done criteria.
2. Inspect codebase context before creating the task.
3. Break the activity into small sequenced tasks.
4. Add assumptions, risks, and rollback notes.
5. Create the task with `orbit task add`.
6. Verify it with `orbit task show <id>`.

## Operating Rules

- Use `orbit task` commands only; do not edit task files directly.
- Never invent task IDs.
- Use explicit flags for each requested field.
- Set `--workspace` to the repo path when available.
- Set attribution fields: `--assigned-to` and `--created-by` when available.
- `description` and `plan` must be multi-line markdown.
- Required fields: `title`, `description`, `plan`, `workspace`, and `proposed-by`.
- `--context` should reference relevant files and task-local artifacts when available.

## Command Reference

```bash
orbit task add \
  --title "<title>" \
  --description "<multi-line markdown>" \
  --plan "<multi-line markdown>" \
  --context "<comma,separated,context>" \
  --workspace "<absolute_or_relative_repo_path>" \
  --assigned-to "<identity_display_name>" \
  --created-by "<identity_display_name>" \
  --priority <low|medium|high|critical> \
  --type <task|feature|issue|chore|refactor> \
  --proposed-by "<identity_display_name>"
```

## Plan Template

```markdown
# <Feature> Implementation Plan

**Goal:** <single sentence>
**Scope:** <what is included/excluded>
**Assumptions:** <key assumptions>
**Risks:** <key technical risks>

## Task 1: <name>

**Files:**
- Create: `path/to/new.file`
- Modify: `path/to/existing.file`
- Test: `path/to/test.file`

**Steps:**
1. Add/adjust failing test(s)
2. Run targeted test: `<command>`
3. Implement minimal change
4. Re-run targeted test: `<command>`
5. Run broader checks: `<command>`

**Done When:**
- <observable condition>

## Task 2: <name>
...

## Final Verification
- `<full test/lint/build commands>`
```

## Exit Criteria

The task exists with required fields and a clear, executable plan.
