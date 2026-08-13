---
name: orbit
description: Entry point for Orbit — tool invocation surfaces, task lifecycle, and routing to the per-topic skills. Also covers first-time setup and orientation when `.orbit/` is absent.
---

# Orbit

Durable operations go through the registered Orbit tool surface — never direct lifecycle subcommands, never a rebuild from source. Lifecycle and authoring detail lives in the per-topic skills listed at the bottom.

If `.orbit/` is absent, or the user is still deciding whether to adopt Orbit, read [references/guide.md](references/guide.md) and follow it end-to-end. Once the workspace is initialized, the rest of this skill applies.

## Tool Invocation

Registered tools are reachable two ways, with identical JSON arguments:

| Surface | Form |
|---------|------|
| **MCP** — Claude Code with the orbit plugin, or any client connected to `orbit mcp serve` | `orbit_task_add({...})` |
| **CLI** — inside an activity step, or with `orbit` on `PATH` | `orbit tool run orbit.task.add --input '{...}'` |

**Mapping rule:** `orbit.<group>.<action>` ↔ `orbit_<group>_<action>` — every dot becomes an underscore, so `orbit.task.artifact.put` is `orbit_task_artifact_put`. A gateway fronting the MCP mirrors this surface exactly; it may add an optional `workspace` routing param and tools of its own, documented by that gateway rather than here.

**Always include `model`** in the JSON — `codex`, `claude`, `gemini`, or `grok` — so Orbit attributes the call. Full model strings auto-normalize; the family is the persisted identity. This holds for every `orbit.*` call in every skill.

`orbit tool list` (CLI) or `tools/list` (MCP) is the authoritative surface; never guess a tool name. Two gaps worth knowing up front:

- **CLI-only:** `orbit semantic`, `orbit.state.*`, and the routine/sweep/job commands — they need direct process access.
- **No query surface at all:** code structure (callers, refs, symbols, implementors). Inspect files with `fs.read`, or `rg` with shell access. Task-to-commit lookup is `git log --grep '[T<task-id>]'`, not a graph query.

Some CLI flags have no MCP counterpart because they are already the MCP default: `orbit tool run orbit.task.show --full` is plain `orbit_task_show({"id": "<id>"})`. Pass `field` or `fields` to project instead — valid values are `comments`, `plan`, `execution_summary`, `description`, `acceptance_criteria`, `history`, `context_files`, `artifacts`. `orbit.task.list` also accepts `path`, returning tasks whose `context_files` selectors cover that path.

If an activity already injected `task` into the execution envelope, read that snapshot instead of calling `orbit.task.show` again.

## Common Mistakes — DO NOT

| Mistake | Why it fails | Correct form |
|---------|-------------|--------------|
| `cargo run -- tool run ...` | Rebuilds from source instead of using the installed binary | `orbit tool run ...` |
| `orbit task show <id>` | Direct CLI subcommands skip agent provenance tracking | `orbit tool run orbit.task.show --input '{"id":"<id>"}'` |

## Lifecycle

```text
proposed → backlog → in-progress → review → done
         ↘ rejected

someday → in-progress
blocked → in-progress

review      → rejected
rejected    → backlog | in-progress  (reconsider)
```

Use `blocked` when execution cannot safely continue. Provenance follows the surface: `orbit tool run ...` is agent-driven, bare `orbit task ...` is human-driven.

## Skill Selection

- `orbit-task`: create, execute, and review tasks through the lifecycle; file self-reported tooling and skill-guidance friction.
- `orbit-task-pilot`: read-only preflight over a bounded partition of existing tasks — proposes `context_files`, crew, blockers, and duplicates without mutating or dispatching anything.
- `orbit-workflow`: jobs, activities, routines, `orbit sweep`, and `orbit run`; diagnosing failed, stuck, or cancelled runs.
- `orbit-search`: find tasks, docs, ADRs, and frictions by topic; docs-corpus admin.
