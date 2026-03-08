---
name: orbit-create-task
description: Use this when you need to create an Orbit task. 
---

# Orbit Create Task

## Purpose

Create an orbit task that another engineer or agent can refer to in order to execute the task without guessing. Keep its plan incremental, and explicit about paths, commands, and expected outcomes.

## Workflow

1. Confirm objective, constraints, and done criteria.
2. Inspect codebase context before creating task.
3. Break job into small tasks with clear sequencing.
4. Add risks, assumptions, and rollback notes.
5. Create an orbit task using `orbit task add` command.

## Operating Rules

- Use `orbit task` commands only. Do not edit backing files directly.
- Never invent task IDs. Resolve IDs from command output or search/list results.
- Use explicit flags for each requested change.
- After create, verify with `orbit task show <id>`.
- Task `--context` should include relevant files and task-local artifact paths when available.
- Task `--workspace` should be set to the repository path when available.
- Always set task attribution fields on create: `--assigned-to` and `--created-by` when available.
- Task `description`, `plan` values must be authored as multi-line markdown content
- Required on create: `title`, `description`, `plan`, `workspace`, and `proposed-by`.


## Command Reference

### Create

```bash
orbit task add \
  --title "<title>" \
  --description "<multi-line markdown content>" \
  --plan "<multi-line markdown content>" \
  --context "<comma,separated,context>" \
  --workspace "<absolute_or_relative_repo_path>" \
  --assigned-to "<identity_display_name_or_model_name>" \
  --created-by "<identity_display_name_or_model_name>" \
  --priority <low|medium|high|critical> \
  --type <task|feature|issue|chore|refactor> \
  --proposed-by "<proposer_name>"
```

## Planning Rules

- Use concrete file paths, not vague references.
- Call out dependencies between tasks.

## Task Plan Template

Use below template to formualate plan for the task.

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

- Orbit task is created with all the required fields, with a detailed plan that can be followed to complete the task successfully.