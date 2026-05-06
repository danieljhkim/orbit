---
name: orbit
description: Entry point for Orbit workflow. Covers tool invocation surfaces, lifecycle, and skill routing. Load this for any Orbit-related work.
---

# Orbit

## Purpose

This skill orients agents working with Orbit. Operations should go through the registered Orbit tool surface — not direct CLI subcommands or rebuilds from source.

Lifecycle and authoring details live in the per-task skills below; this skill stays brief on purpose.

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

**Always include `model` in the JSON** so Orbit can attribute the call to the right agent family:

```json
{ "model": "<model_name>" }
```

**CLI-flag → JSON mapping:** the CLI exposes some flags (e.g. `orbit tool run orbit.task.show --full ...`) that don't appear over MCP. The MCP equivalent is the default behavior when the corresponding JSON field is omitted (e.g. `orbit_task_show({"id": "<id>"})` returns the full task; pass `field` or `fields` to project).

Examples below use CLI form for readability; substitute the MCP form using the mapping above when MCP tools are loaded.

## Common Command Reference

The reference below is intentionally common, not exhaustive. Never guess. Run `orbit tool list` (CLI) or call `tools/list` (MCP) to see the full registered tool surface. If an activity already injected `task` into the execution envelope, use that snapshot instead of calling `orbit.task.show` again.

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
# Review-thread commands: add/reply require `model`; list/resolve show it for provenance consistency, though it is optional there.
orbit tool run orbit.task.review_thread.add --input '{"id": "<id>", "body": "...", "path": "<repo-relative path>", "line": "<line>", "model": "<model_name>"}'
orbit tool run orbit.task.review_thread.list --input '{"id": "<id>", "status": "open", "model": "<model_name>"}'
orbit tool run orbit.task.review_thread.reply --input '{"id": "<id>", "thread_id": "<thread-id>", "body": "...", "model": "<model_name>"}'
orbit tool run orbit.task.review_thread.resolve --input '{"id": "<id>", "thread_id": "<thread-id>", "model": "<model_name>"}'
orbit tool run orbit.task.locks.reserve --input '{"files": ["file:src/foo.rs"], "model": "<model_name>"}'
orbit tool run orbit.task.locks.release --input '{"reservation_id": "<reservation-id>", "model": "<model_name>"}'
```

## Common Mistakes — DO NOT

| Mistake | Why it fails | Correct form |
|---------|-------------|--------------|
| `cargo run -- tool run ...` | Agents must use the installed `orbit` binary, not rebuild from source | `orbit tool run ...` |
| `orbit task show <id>` | Direct CLI subcommands skip agent provenance tracking | `orbit tool run orbit.task.show --full --input '{"id":"<id>"}'` |

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
- `orbit-locks`: Reserve and release file locks for ad-hoc code modification outside workflow-held reservations.
- `orbit-review-task`: Review someone else's work and file findings as review threads, without transitioning the task.
- `orbit-track-issues`: Capture agent-discovered, self-reported friction as tracked tasks.
- `orbit-graph`: Navigate or inspect the codebase via the knowledge graph when the activity allowlist includes graph tools.

## Voice Your Opinion

If something is unclear, missing, buggy, or creates friction during agent work, track it with `orbit-track-issues`. Reserve task type `friction` for that self-report path only.
