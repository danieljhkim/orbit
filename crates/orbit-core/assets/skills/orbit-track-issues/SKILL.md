---
name: orbit-track-issues
description: MUST use this skill when Orbit tooling or skill instructions cause operational friction (tool failures, wrong CLI behavior, misleading skill guidance). Not for task content issues like vague descriptions or incomplete plans.
---

# Orbit Skill: Track Issues

## Purpose

This skill ensures that **agent-discovered Orbit tooling, workflow, or seeded-instruction friction** is recorded as an append-only Orbit friction report.

Orbit is designed to continuously improve. When agents encounter problems, they must **create reports instead of silently working around them**.
These reports are reserved for self-reported agent friction. Do not use this skill for ordinary user-requested work, generic bugs, or backlog items.

Examples of issues worth tracking:

- unclear command behavior
- missing CLI functionality
- confusing schema or config
- documentation gaps
- unclear error messages
- unexpected runtime behavior
- confusing seed instructions

If Orbit tooling or Orbit-authored guidance slows the agent down, it should be tracked.

## Storage

Every self-reported friction record is stored under `.orbit/frictions/` as append-only markdown with YAML frontmatter.
There is no task lifecycle, triage state, rejection penalty, or precomputed scoreboard file.

Do **not ignore friction**. Always create a report.

## Valid Tags

| Tag | Use for |
| --- | --- |
| `build` | make/fmt/lint friction |
| `docs` | Stale or missing CLAUDE.md or design docs |
| `lifecycle` | Task lifecycle confusion or transition issues |
| `naming` | Naming drift or duplicated sources of truth |
| `other` | Fallback when no specific tag fits |
| `policy` | fsProfile or sandboxing surprises |
| `skill-guidance` | Misleading or incorrect skill instructions |
| `tooling` | Orbit tool/CLI/MCP failures |

## How to Create the Report

Two surfaces, identical JSON args. See the `orbit` skill for the full mapping.

CLI form:

```bash
orbit tool run orbit.friction.add --input '{
  "body": "<what happened, where, and why it caused friction>",
  "tags": ["<tooling|skill-guidance|docs|lifecycle|build|naming|policy|other>"],
  "during_task": "<optional task id>",
  "model": "<family>" # codex | claude | gemini | grok (full model strings are accepted and auto-normalized)
}'
```

MCP form (same JSON, called as `orbit_friction_add`):

```text
orbit_friction_add({
  "body": "<what happened, where, and why it caused friction>",
  "tags": ["<tooling|skill-guidance|docs|lifecycle|build|naming|policy|other>"],
  "during_task": "<optional task id>",
  "model": "<family>"  // codex | claude | gemini | grok
})
```

Keep the description concrete — name the command, file, or workflow that broke.

## Important Rules

- Do not silently ignore Orbit problems — always create a report.
- Do not implement large design changes inline — track them first.
- Document the root issue clearly so the next agent can act on it.
- Report genuine friction only.
