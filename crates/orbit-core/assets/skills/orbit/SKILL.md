---
name: orbit
description: Entry point for Orbit workflow. Covers lifecycle, invocation patterns, and skill routing. Load this for any Orbit-related work.
---

# Orbit

## Purpose

This skill orients agents working with Orbit. Orbit operations should go through the registered Orbit tool surface.

## Tool Invocation

Orbit tools are reachable via two surfaces. Both accept identical JSON arguments.

| Surface | When to use | Form |
|---------|-------------|------|
| **MCP** | Claude Code with the orbit plugin (or any MCP client connected to `orbit mcp serve`); look for `orbit_*` tools in your toolbox | `orbit_task_add({"title": "...", "model": "<model_name>"})` |
| **CLI** | Shell access (inside an activity step, or with the `orbit` binary on `PATH`) | `orbit tool run orbit.task.add --input '{"title": "...", "model": "<model_name>"}'` |

**Mapping rule**: `orbit.<group>.<action>` ↔ `orbit_<group>_<action>` (dots become underscores; JSON args identical). For multi-segment names like `orbit.task.review_thread.add`, every dot becomes an underscore: `orbit_task_review_thread_add`.

**Surface coverage:**

- Task lifecycle (`orbit.task.*`): both surfaces.
- Graph read tools (`search`, `show`, `pack`, `callers`, `refs`, `implementors`, `deps`, `overview`, `history`): both surfaces.
- State handoff (`orbit.state.*`), graph writes, and duel/scoreboard tools: **CLI only** — used inside activity steps where the agent has shell access.

**Always include `model` in the JSON** (both surfaces) so Orbit can attribute the call to the right agent family:

```json
{ "model": "<model_name>" }
```

**CLI-flag → JSON mapping:** the CLI exposes some flags (e.g. `orbit tool run orbit.task.show --full ...`) that don't appear over MCP. The MCP equivalent is the default behavior when the corresponding JSON field is omitted (e.g. `orbit_task_show({"id": "<id>"})` returns the full task; pass `field` or `fields` to project).

Examples below use CLI form for readability; substitute the MCP form using the mapping above when MCP tools are loaded.

## Common Workflows

### Loading and executing a task

**Inside `agent_implement` or any activity that injects `task` into the execution envelope:** use the injected `task.*` fields directly. Do not call `orbit.task.show` unless the activity instructions explicitly require it and the tool appears in the activity allowlist.

1. If the activity did not preload `task`, load the task: `orbit tool run orbit.task.show --full --input '{"id": "<task-id>", "model": "<model_name>"}'`
2. Read the `description` and `acceptance_criteria` first — they define the required outcome.
3. If the `plan` field is blank or placeholder text, author a fresh plan with `orbit.task.update`.
4. Treat `context_files` as selector-first task context. Prefer canonical selectors (`file:`, `dir:`, `symbol:`), use `orbit.graph.pack` when available, and only fall back to direct file reads for unresolved selectors or when the graph is unavailable.
5. Start the task: `orbit tool run orbit.task.start --input '{"id": "<task-id>", "note": "...", "model": "<model_name>"}'`
6. Implement following the plan. Validate using the plan's verification steps.
7. Move to review: `orbit tool run orbit.task.update --input '{"id": "<task-id>", "status": "review", "model": "<model_name>"}'`

### Reporting progress or problems

- Add a comment: `orbit tool run orbit.task.update --input '{"id": "<task-id>", "comment": "what happened", "model": "<model_name>"}'`
- If execution fails, comment with what went wrong before stopping. The next agent needs this context.

### Finding work

- List backlog: `orbit tool run orbit.task.list --input '{"status": "backlog"}'`
- List in review: `orbit tool run orbit.task.list --input '{"status": "review"}'`
- Search by text: `orbit tool run orbit.task.search --input '{"query": "search text", "model": "<model_name>"}'`

### Passing state between steps

Use `orbit.state.*` for data that must flow from one activity/job step to a later step.
Do not rely on the final activity response payload as the handoff mechanism.

- `orbit.state.get` reads the persisted pipeline snapshot.
- `orbit.state.set` writes this step's output for the engine to merge after the step finishes.
- Once the needed fields are written to `orbit.state`, there should usually be no structured response-payload requirement for the activity itself.
- Continue using `orbit.task.update` for task artifacts like `execution_summary`, `pr_status`, comments, and lifecycle state. That is task persistence, not pipeline-state handoff.
- Only call `orbit.state.*` when the activity allowlist includes those tools.

Concrete examples:

```bash
# Reviewer step: persist review data for downstream arbitration
orbit tool run orbit.state.set --input '{
  "data": {
    "decision": "request-changes",
    "threads": [
      {"id": "thread-1", "path": "src/lib.rs", "line": 42, "body": "Missing null check."}
    ],
    "summary": "One blocking correctness issue remains."
  }
}'

# Arbiter step: recover review threads if they were not injected into input
orbit tool run orbit.state.get --input '{"key": "threads"}'

# Arbiter step: persist verdict fields for gate + scoreboard steps
orbit tool run orbit.state.set --input '{
  "data": {
    "decision": "APPROVED",
    "reviewer_score": 4.0,
    "implementer_score": 4.5,
    "blocking_comment_ids": [],
    "task_class_ambiguity": "well_specified"
  }
}'
```

For `run_command` or any shell-based step, there is no implicit structured output path anymore beyond `exit_code`. If the command must feed downstream steps, have it invoke `orbit.state.set` explicitly from the command it runs. Downstream jobs should read the persisted state, not depend on the shell step returning structured JSON.

## Common Command Reference

Invoke Orbit through `orbit tool run`:

If an activity already injected `task` into the execution envelope, use that snapshot instead of calling `orbit.task.show` again.

```bash
# Task commands
orbit tool run orbit.task.show --full --input '{"id": "<id>", "model": "<model_name>"}'                    # Load full task
orbit tool run orbit.task.show --input '{"id": "<id>", "field": "comments", "model": "<model_name>"}'     # Load only comments
orbit tool run orbit.task.show --input '{"id": "<id>", "field": "plan", "model": "<model_name>"}'         # Load only plan
# Valid field values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts
orbit tool run orbit.task.list --input '{"status": "backlog", "model": "<model_name>"}'       # List by status
orbit tool run orbit.task.search --input '{"query": "search text", "model": "<model_name>"}'  # Search title/description text
orbit tool run orbit.task.add --input '{"title": "...", "description": "...", "acceptance_criteria": ["..."], "workspace": ".", "model": "<model_name>"}'
orbit tool run orbit.task.update --input '{"id": "<id>", "plan": "...", "model": "<model_name>"}'
orbit tool run orbit.task.start --input '{"id": "<id>", "note": "...", "model": "<model_name>"}' # backlog -> in-progress
orbit tool run orbit.task.update --input '{"id": "<id>", "status": "review", "model": "<model_name>"}'
orbit tool run orbit.task.update --input '{"id": "<id>", "comment": "...", "model": "<model_name>"}'
orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "...", "model": "<model_name>"}' # proposed/friction -> backlog, review -> done
orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "...", "model": "<model_name>"}'   # proposed/friction -> rejected
orbit tool run orbit.task.locks --input '{}'                         # View active file locks
orbit tool run orbit.task.review_thread.add --input '{"id": "<id>", "body": "..."}'
orbit tool run orbit.task.review_thread.list --input '{"id": "<id>", "status": "open"}'
orbit tool run orbit.task.review_thread.reply --input '{"id": "<id>", "thread_id": "<thread-id>", "body": "..."}'
orbit tool run orbit.task.review_thread.resolve --input '{"id": "<id>", "thread_id": "<thread-id>"}'

# State handoff commands
orbit tool run orbit.state.get --input '{"key": "decision"}'
orbit tool run orbit.state.get --input '{}'
orbit tool run orbit.state.set --input '{"key": "decision", "value": "APPROVED"}'
orbit tool run orbit.state.set --input '{"data": {"threads": [], "summary": "Looks good"}}'

```


## Common Mistakes — DO NOT

| Mistake | Why it fails | Correct form |
|---------|-------------|--------------|
| `cargo run -- tool run ...` | Agents must use the installed `orbit` binary, not rebuild from source | `orbit tool run ...` |
| `orbit task show <id>` | Direct CLI subcommands skip agent provenance tracking | `orbit tool run orbit.task.show --full --input '{"id":"<id>"}'` |

**Rule:** The command reference above is intentionally common, not exhaustive. Never guess. Run `orbit tool list` (CLI) or call `tools/list` (MCP) to see the full registered tool surface.

## Lifecycle

```text
proposed → backlog → in-progress → review → done
         ↘ rejected
friction → backlog | in-progress | done
         ↘ rejected

someday → in-progress
blocked → in-progress
```

Rejection path:

```text
review      → rejected
rejected    → backlog | in-progress  (reconsider)
```

Use `blocked` when execution cannot safely continue.

Command surface determines provenance by default:
- `orbit tool run ...` is treated as agent-driven
- direct `orbit task ...` CLI usage is treated as human-driven

## Skill Selection

- `orbit-create-task`: Create a new task with description, acceptance criteria, and context.
- `orbit-execute-task`: Carry a change through implementation, validation, and review.
- `orbit-review-task`: Review someone else's work and file findings as review threads, without transitioning the task.
- `orbit-track-issues`: Capture agent-discovered, self-reported friction as tracked tasks.
- `orbit-graph`: Navigate or inspect the codebase via the knowledge graph when the activity allowlist includes graph tools.

## Voice Your Opinion

If something is unclear, missing, bugs or creates friction during agent work, track it with `orbit-track-issues`. Reserve task type `friction` for that self-report path only.
