---
name: orbit
description: Entry point for Orbit workflow. Covers lifecycle, invocation patterns, and skill routing. Load this for any Orbit-related work.
---

# Orbit

## Purpose

This skill orients agents working with Orbit. Orbit operations should go through the registered Orbit tool surface.

## Common Workflows

### Loading and executing a task

**Inside `implement_change` (and any activity that injects `task` into the execution envelope):** use the injected `task.*` fields directly. Do not call `orbit.task.show` unless the activity instructions explicitly require it and the tool appears in the activity allowlist.

1. If the activity did not preload `task`, load the task: `orbit tool run orbit.task.show --input '{"id": "<task-id>"}'`
2. Read the `description` and `acceptance_criteria` first — they define the required outcome.
3. If the `plan` field is blank or placeholder text, author a fresh plan with `orbit.task.update`.
4. Read each file listed in `context_files` before making changes.
5. Start the task: `orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "..."}'`
6. Implement following the plan. Validate using the plan's verification steps.
7. Move to review: `orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review"}'`

### Reporting progress or problems

- Add a comment: `orbit tool run orbit.task.update --input '{"id": "<task-id>", "comment": "what happened"}'`
- If execution fails, comment with what went wrong before stopping. The next agent needs this context.

### Finding work

- List backlog: `orbit tool run orbit.task.list --input '{"status": "backlog"}'`
- List in review: `orbit tool run orbit.task.list --input '{"status": "review"}'`

## Command Reference

Invoke Orbit through `orbit tool run`:

If an activity already injected `task` into the execution envelope, use that snapshot instead of calling `orbit.task.show` again.

```bash
# Task commands
orbit tool run orbit.task.show --input '{"id": "<id>"}'                          # Load full task
orbit tool run orbit.task.show --input '{"id": "<id>", "field": "comments"}'     # Load only comments
orbit tool run orbit.task.show --input '{"id": "<id>", "field": "plan"}'         # Load only plan
# Valid field values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files
orbit tool run orbit.task.list --input '{"status": "backlog"}'       # List by status
orbit tool run orbit.task.add --input '{"title": "...", "description": "...", "acceptance_criteria": ["..."], "workspace": "."}'
orbit tool run orbit.task.update --input '{"id": "<id>", "plan": "..."}'
orbit tool run orbit.task.start --input '{"id": "<id>", "note": "..."}' # backlog -> in-progress
orbit tool run orbit.task.update --input '{"id": "<id>", "status": "review"}'
orbit tool run orbit.task.update --input '{"id": "<id>", "comment": "..."}'
orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "..."}'
orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "..."}'

```


## Common Mistakes — DO NOT

| Mistake | Why it fails | Correct form |
|---------|-------------|--------------|
| `cargo run -- tool run ...` | Agents must use the installed `orbit` binary, not rebuild from source | `orbit tool run ...` |
| `orbit task show <id>` | Direct CLI subcommands skip agent provenance tracking | `orbit tool run orbit.task.show --input '{"id":"<id>"}'` |

**Rule:** If a tool name is not in the Command Reference, it does not exist. Never guess. Run `orbit tool list` to see all registered tools.

## Lifecycle

```text
proposed → backlog → in-progress → review → done
```

Rejection path:

```text
proposed → rejected
review    → rejected
rejected  → backlog  (reconsider)
```

Use `blocked` when execution cannot safely continue.

Task commands infer actor provenance automatically:
- `orbit tool run ...` is treated as agent-driven
- direct `orbit task ...` CLI usage is treated as human-driven

## Skill Selection

- `orbit-create-task`: Create a new task with description, acceptance criteria, and context.
- `orbit-approve-task`: Record human approval or rejection at lifecycle gates.
- `orbit-execute-task`: Carry a change through implementation, validation, and review.
- `orbit-pr`: Create, review, and discuss pull requests.
- `orbit-track-issues`: Capture agent-discovered, self-reported friction as tracked tasks.
- `orbit-graph`: Navigate, inspect, or edit orbit-harnessed codebase via the knowledge graph

## Voice Your Opinion

If something is unclear, missing, bugs or creates friction during agent work, track it with `orbit-track-issues`. Reserve task type `friction` for that self-report path only.
