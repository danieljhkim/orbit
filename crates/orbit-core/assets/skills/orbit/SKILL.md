---
name: orbit
description: Entry point for Orbit workflow once a workspace is initialized (`.orbit/` present). Covers tool invocation surfaces, lifecycle, and skill routing among workflow siblings (`orbit-create-task`, `orbit-execute-task`, `orbit-review-task`, etc.). For first-time setup, install, or "what is orbit" tour requests, defer to `orbit-guide`.
---

# Orbit

## Purpose

This skill orients agents working with Orbit. Operations should go through the registered Orbit tool surface — not direct CLI subcommands or rebuilds from source.

Lifecycle and authoring details live in the per-task skills below; this skill stays brief on purpose.

## Tool Invocation

Orbit tools are reachable via two surfaces. Both accept identical JSON arguments.

| Surface | When to use | Form |
|---------|-------------|------|
| **MCP** | Claude Code with the orbit plugin (or any MCP client connected to `orbit mcp serve`); look for `orbit_*` tools in your toolbox | `orbit_task_add({"title": "...", "model": "<agent-family>"})` |
| **CLI** | Shell access (inside an activity step, or with the `orbit` binary on `PATH`) | `orbit tool run orbit.task.add --input '{"title": "...", "model": "<agent-family>"}'` |

**Mapping rule**: `orbit.<group>.<action>` ↔ `orbit_<group>_<action>` (dots become underscores; JSON args identical). For multi-segment names like `orbit.task.review_thread.add`, every dot becomes an underscore: `orbit_task_review_thread_add`.

**Surface coverage:**

- Task lifecycle (`orbit.task.*`): both surfaces.
- ADR artifacts (`orbit.adr.*`): both surfaces.
- Graph read tools (`search`, `show`, `pack`, `callers`, `refs`, `implementors`, `deps`, `overview`, `history`): both surfaces.
- Semantic read tools (`orbit.semantic.search`, `orbit.semantic.related`): both surfaces. Require the `orbit-embed-companion` binary (`orbit semantic install`); calls fail with an install-pointer error otherwise.
- State handoff (`orbit.state.*`), graph writes, and duel/scoreboard tools: **CLI only** — used inside activity steps where the agent has shell access.

**Always include `model` in the JSON** so Orbit can attribute the call to the right agent family. Here `model` means the canonical agent family: pass `codex`, `claude`, `gemini`, or `grok`. Full model strings are accepted and auto-normalized, but the family is the persisted identity.

```json
{ "model": "codex" }
```

**CLI-flag → JSON mapping:** the CLI exposes some flags (e.g. `orbit tool run orbit.task.show --full ...`) that don't appear over MCP. The MCP equivalent is the default behavior when the corresponding JSON field is omitted (e.g. `orbit_task_show({"id": "<id>"})` returns the full task; pass `field` or `fields` to project).

Examples below use CLI form for readability; substitute the MCP form using the mapping above when MCP tools are loaded.

## Common Command Reference

The reference below is intentionally common, not exhaustive. Never guess. Run `orbit tool list` (CLI) or call `tools/list` (MCP) to see the full registered tool surface. If an activity already injected `task` into the execution envelope, use that snapshot instead of calling `orbit.task.show` again.

```bash
# Task commands
orbit tool run orbit.task.show --full --input '{"id": "<id>", "model": "<agent-family>"}'                    # Load full task
orbit tool run orbit.task.show --input '{"id": "<id>", "field": "comments", "model": "<agent-family>"}'     # Load only comments
orbit tool run orbit.task.show --input '{"id": "<id>", "field": "plan", "model": "<agent-family>"}'         # Load only plan
# Valid field values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts
orbit tool run orbit.task.list --input '{"status": "backlog", "model": "<agent-family>"}'       # List by status
orbit tool run orbit.task.search --input '{"query": "search text", "model": "<agent-family>"}'  # Lexical title/description substring match
orbit tool run orbit.semantic.search --input '{"query": "topic phrase", "limit": 5, "model": "<agent-family>"}'  # Hybrid BM25 + cosine over indexed task fields (requires `orbit semantic install`)
orbit tool run orbit.semantic.related --input '{"id": "<task-id>", "limit": 5, "model": "<agent-family>"}'        # Cosine neighbors of an indexed task
orbit tool run orbit.task.add --input '{"title": "...", "description": "...", "acceptance_criteria": ["..."], "workspace": ".", "model": "<agent-family>"}'
orbit tool run orbit.task.update --input '{"id": "<id>", "plan": "...", "model": "<agent-family>"}'
orbit tool run orbit.task.start --input '{"id": "<id>", "note": "...", "model": "<agent-family>"}' # backlog -> in-progress
orbit tool run orbit.task.update --input '{"id": "<id>", "status": "review", "model": "<agent-family>"}'
orbit tool run orbit.task.update --input '{"id": "<id>", "comment": "...", "model": "<agent-family>"}'
orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "...", "model": "<agent-family>"}' # proposed/friction -> backlog, review -> done
orbit tool run orbit.task.reject --input '{"id": "<id>", "note": "...", "model": "<agent-family>"}'   # proposed/friction -> rejected
# Review-thread commands: add/reply require `model`; list/resolve show it for provenance consistency, though it is optional there.
orbit tool run orbit.task.review_thread.add --input '{"id": "<id>", "body": "...", "path": "<repo-relative path>", "line": "<line>", "model": "<agent-family>"}'
orbit tool run orbit.task.review_thread.list --input '{"id": "<id>", "status": "open", "model": "<agent-family>"}'
orbit tool run orbit.task.review_thread.reply --input '{"id": "<id>", "thread_id": "<thread-id>", "body": "...", "model": "<agent-family>"}'
orbit tool run orbit.task.review_thread.resolve --input '{"id": "<id>", "thread_id": "<thread-id>", "model": "<agent-family>"}'
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
- `orbit-adr`: Create, update, inspect, accept, or supersede ADR artifacts through `orbit.adr.*`.
- `orbit-design`: Scaffold, list, or inspect `docs/design/<feature>/` folders through `orbit.design.*`. Use before authoring a new feature folder and before declaring a doc current.
- `orbit-docs`: Search, show, register, reindex, or migrate the human-authored docs corpus through `orbit.docs.*`. Use for docs retrieval over `docs/`; ADRs remain owned by `orbit-adr`.
- `orbit-debug-job-failure`: Diagnose failed, stuck, cancelled, or suspicious Orbit job runs.
- `orbit-execute-task`: Carry a change through implementation, validation, and review.
- `orbit-review-task`: Review someone else's work and file findings as review threads, without transitioning the task.
- `orbit-learning`: Author, search, update, supersede, and prune project learnings through `orbit.learning.*`. Use to preserve recurring gotchas, incident root-causes, and cross-session guidance.
- `orbit-track-issues`: Capture agent-discovered, self-reported friction as append-only reports.
- `orbit-graph`: Navigate or inspect the codebase via the knowledge graph when the activity allowlist includes graph tools.
- `orbit-semantic`: Find tasks by topic — pre-create dedup checks, related-task lookups, "didn't we have a task about X?" queries. Complementary to `orbit-graph` (code structure vs task content).
- `orbit-guide`: First-time onboarding when `.orbit/` is absent, or feature-tour requests ("what is orbit", "give me a tour"). Walks setup paths and hands off to `orbit-create-task`.

## Voice Your Opinion

If something is unclear, missing, buggy, or creates friction during agent work, track it with `orbit-track-issues`.
