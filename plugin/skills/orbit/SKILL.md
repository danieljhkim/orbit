---
name: orbit
description: Entry point for Orbit. Once a workspace is initialized (`.orbit/` present), routes to workflow siblings (`orbit-task`, `orbit-workflow`, `orbit-search`, `orbit-knowledge`) and covers tool invocation surfaces and task lifecycle. When `.orbit/` is absent, or for first-time setup/install ("set up orbit", "install orbit", "wire orbit into this repo", "I'm new to orbit") or feature-tour requests ("what is orbit", "give me a tour"), see references/guide.md.
---

# Orbit

## Purpose

This skill orients agents working with Orbit. Operations should go through the registered Orbit tool surface — not direct CLI subcommands or rebuilds from source.

Lifecycle and authoring details live in the per-topic skills below; this skill stays brief on purpose.

## First-Time Setup / Tour

If `.orbit/` is absent in the current workspace, or the user asks "what is orbit" / "give me a tour" / hasn't committed to using it yet, read [references/guide.md](references/guide.md) and follow it end-to-end (detect state → pick setup path → run setup → verify → hand off). Once the workspace is initialized, the rest of this skill applies.

## Tool Invocation

Orbit tools are reachable via two surfaces. Both accept identical JSON arguments.

| Surface | When to use | Form |
|---------|-------------|------|
| **MCP** | Claude Code with the orbit plugin (or any MCP client connected to `orbit mcp serve`); look for `orbit_*` tools in your toolbox | `orbit_task_add({"title": "...", "model": "<agent-family>"})` |
| **CLI** | Shell access (inside an activity step, or with the `orbit` binary on `PATH`) | `orbit tool run orbit.task.add --input '{"title": "...", "model": "<agent-family>"}'` |

**Mapping rule**: `orbit.<group>.<action>` ↔ `orbit_<group>_<action>` (dots become underscores; JSON args identical). For multi-segment names like `orbit.task.review_thread.add`, every dot becomes an underscore: `orbit_task_review_thread_add`.

**Environment parity**: the orbit tool surface is identical across MCP and CLI. Some deployments front the orbit MCP behind a gateway that mirrors the surface exactly and may add an optional `workspace` routing param on tools that lack one. Any non-standard tools a gateway adds beyond the orbit surface are documented by that gateway, not here — consult its own reference before relying on them.

**Surface coverage:**

- Task lifecycle (`orbit.task.*`), ADR artifacts (`orbit.adr.*`), learnings (`orbit.learning.*`), unified search (`orbit.search`): both surfaces.
- Graph tools (`sync`, `search`, `show`, `refs`, `callees`, `impact`, `trace`, `overview`, `implementors`, `deps`): **MCP only** — served in-process by orbit-graph (v2). When present in your tool surface they are self-describing; no prompt-side instruction is needed. Task-to-commit lookup is not a graph tool; use `git log --grep '[T<task-id>]'`.
- Semantic lifecycle (`orbit semantic install|stats|index|uninstall`) and state handoff (`orbit.state.*`), routine/sweep/job commands: **CLI only** — used inside activity steps or from a shell where the agent has direct process access.

**Always include `model` in the JSON** so Orbit can attribute the call to the right agent family: `codex`, `claude`, `gemini`, or `grok`. Full model strings are accepted and auto-normalized, but the family is the persisted identity.

**CLI-flag → JSON mapping:** the CLI exposes some flags (e.g. `orbit tool run orbit.task.show --full ...`) that don't appear over MCP. The MCP equivalent is the default behavior when the corresponding JSON field is omitted (e.g. `orbit_task_show({"id": "<id>"})` returns the full task; pass `field` or `fields` to project).

Examples below use CLI form for readability; substitute the MCP form using the mapping above when MCP tools are loaded.

## Common Command Reference

Intentionally common, not exhaustive. Never guess — run `orbit tool list` (CLI) or call `tools/list` (MCP) for the full registered tool surface. If an activity already injected `task` into the execution envelope, use that snapshot instead of calling `orbit.task.show` again.

```bash
orbit tool run orbit.task.show --full --input '{"id": "<id>", "model": "<agent-family>"}'                    # Load full task
orbit tool run orbit.task.show --input '{"id": "<id>", "field": "comments", "model": "<agent-family>"}'       # Load one field
# Valid field values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts
orbit tool run orbit.task.list --input '{"status": "backlog", "model": "<agent-family>"}'
orbit tool run orbit.task.list --input '{"path": "src/auth/login.rs", "model": "<agent-family>"}'             # Tasks whose context_files apply to this path
orbit tool run orbit.search --input '{"query": "topic phrase", "kind": "task", "limit": 5, "model": "<agent-family>"}'
orbit tool run orbit.search --input '{"query": "topic phrase", "hybrid": true, "kind": "task", "limit": 5, "model": "<agent-family>"}'
orbit tool run orbit.task.add --input '{"title": "...", "description": "...", "acceptance_criteria": ["..."], "workspace": ".", "model": "<agent-family>"}'
orbit tool run orbit.task.update --input '{"id": "<id>", "plan": "...", "model": "<agent-family>"}'
orbit tool run orbit.task.start --input '{"id": "<id>", "note": "...", "model": "<agent-family>"}'            # backlog -> in-progress
orbit tool run orbit.task.approve --input '{"id": "<id>", "note": "...", "model": "<agent-family>"}'          # proposed -> backlog, review -> done
orbit tool run orbit.task.review_thread.add --input '{"id": "<id>", "body": "...", "path": "<path>", "line": "<line>", "model": "<agent-family>"}'
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

someday → in-progress
blocked → in-progress

review      → rejected
rejected    → backlog | in-progress  (reconsider)
```

Use `blocked` when execution cannot safely continue. Command surface determines provenance by default: `orbit tool run ...` is agent-driven, direct `orbit task ...` CLI usage is human-driven.

## Skill Selection

- `orbit-task`: Create, execute, and review tasks through the full lifecycle; also files self-reported tooling/skill-guidance friction.
- `orbit-workflow`: Jobs, activities, routines, `orbit sweep`, and `orbit run`; diagnosing failed/stuck/cancelled job runs.
- `orbit-search`: Find tasks, docs, learnings, and ADRs by topic; docs-corpus admin (list/show/add root/index/migrate).
- `orbit-knowledge`: Author, update, and supersede learnings and ADRs.

## Voice Your Opinion

If something is unclear, missing, buggy, or creates friction during agent work, track it with `orbit-task` (friction reporting).
