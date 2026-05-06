---
title: Configuration
description: "config.toml file locations, shape, and backend precedence."
sidebar:
  order: 5
---

## File Locations

| Path | Scope |
|------|-------|
| `~/.orbit/config.toml` | Global defaults |
| `.orbit/config.toml` | Workspace-local |

The workspace config **replaces** the global config when present — it does not merge. Move settings into the workspace file or rely on the global file alone.

`orbit init` seeds the global config and prompts for per-role agent settings interactively. Re-run `orbit init --force` for a fresh prompt; edit the file directly to tweak values without re-prompting.

## Shape

```toml
[execution.env]
inherit = false
pass = ["HOME", "PATH", "CODEX_HOME", "TMPDIR", "USER"]

[task.approval]
required_for_agent = true
delegate_approval = true

[graph]
editing = false

# Per-role agent settings (provider: claude | codex | gemini)
[agent.reviewer]
provider = "claude"
backend = "cli"
model = "claude-opus-4-7"

[agent.implementer]
provider = "codex"
backend = "cli"
model = "gpt-5.5"

[agent.planner]
provider = "claude"
backend = "cli"
model = "claude-opus-4-7"
```

The three roles — `reviewer`, `implementer`, and `planner` — are referenced by the seeded workflows. `backend = "cli"` is the only release-supported value in v1.

## Root Override

Most commands accept the global `--root` option to override the Orbit root directory.

```bash
orbit --root /path/to/orbit-root task list
```

## Backend Precedence

For `agent_loop` execution, backend selection resolves once before dispatch.

1. command flag (`--backend`)
2. `ORBIT_BACKEND`
3. `[runtime] backend`
4. hard-coded fallback: **`http`**

**v1 release scope.** v1 supports `backend: cli` only, but the hard-coded fallback is `http` — so omitting all four tiers will silently land on the preview HTTP transport. Pin `cli` explicitly:

```toml
# config.toml
[runtime]
backend = "cli"
```

…or via environment for one-off invocations:

```bash
ORBIT_BACKEND=cli orbit run job task_auto_pipeline
```

Accepted backend values:

| Value | v1 status |
|-------|-----------|
| `cli` | Supported. The v1 release path. |
| `http` | Preview / not in v1 release surface. Wired in code for v2; do not depend on its behavior in v1. |
| `auto` | Resolves to a concrete backend at load time. Always pin `cli` explicitly in v1 instead of relying on `auto`. |

## Workspace State

Workspace-local state lives under `.orbit/` in the repository. Global state is initialized with `orbit init`, usually under `~/.orbit/`.
