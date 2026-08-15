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

`orbit init` seeds the global config with crews for detected provider CLIs.
Re-run `orbit init --force` to reset the global root before initialization.

## Shape

```toml
[execution.env]
pass = ["HOME", "PATH", "CODEX_HOME", "TMPDIR", "USER"]

[execution.codex]
sandbox = "danger-full-access"

[crews.terra]
provider = "codex"
backend = "cli"
model = "gpt-5.6-terra"

[workflow]
base_branch = "main"
default_crew = "terra"

[runtime]
backend = "cli"
```

Each crew is one provider/model/backend assignment. Supported CLI-executable
provider families are `claude`, `codex`, `gemini`, and `grok`. Tasks may select
a named crew, otherwise `[workflow].default_crew` is used.

## Root Override

Most commands accept the global `--root` option to override the Orbit root directory.

```bash
orbit --root /path/to/orbit-root task list
```

## Retired Backend Selection

`agent_loop` execution runs through the provider's CLI agent. The `--backend`
flag, `ORBIT_BACKEND`, `[runtime] backend`, and `[crews.<name>] backend` were
removed: there is no backend left to select.

Existing declarations are recognized rather than ignored, so nothing is
silently reinterpreted. `cli` is accepted and inert; `http` and `auto` are
rejected with the migration message. Remove the setting.

## Workspace State

Workspace-local state lives under `.orbit/` in the repository. Global state is initialized with `orbit init`, usually under `~/.orbit/`.
