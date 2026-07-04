---
title: Architecture
description: "Orbit's crate boundaries and mirrored architecture design documentation."
sidebar:
  order: 1
---

## Crate Graph

Orbit is a layered Rust workspace. Lower layers do not depend on higher layers.

```mermaid
flowchart LR
  Common["orbit-common"] --> Policy["orbit-policy"]
  Common --> Exec["orbit-exec"]
  Common --> Knowledge["orbit-knowledge"]
  Common --> Store["orbit-store"]
  Common --> MCP["orbit-mcp"]
  Policy --> Tools["orbit-tools"]
  Exec --> Tools
  Knowledge --> Tools
  Tools --> Agent["orbit-agent"]
  Tools --> Engine["orbit-engine"]
  Store --> Engine
  Agent --> Engine
  Engine --> Core["orbit-core"]
  Core --> Cmd["orbit-cmd"]
  Core --> CLI["orbit-cli"]
  Cmd --> CLI
  MCP --> CLI
```

`orbit-store` and `orbit-mcp` depend only on `orbit-common`. `orbit-mcp` is consumed by `orbit-cli` directly via `orbit mcp serve`. `orbit-core` does **not** depend on `orbit-agent`; the bridge is `orbit-engine`'s `backend: cli` subprocess runner.

## Boundaries

| Crate | Role |
|-------|------|
| `orbit-common` | Shared domain types, errors, IDs, utility helpers. |
| `orbit-knowledge` | Graph parsing, storage helpers, selectors, graph services. |
| `orbit-store` | YAML and SQLite stores. |
| `orbit-agent` | HTTP loop transport and retained CLI runtimes. |
| `orbit-engine` | Activity/job execution, template rendering, retries, CLI subprocess runner. |
| `orbit-tools` | Built-in tool registry and external tool integration. |
| `orbit-mcp` | MCP adapter over the tool registry. |
| `orbit-core` | Runtime bootstrap, config, runtime-integrated commands, default asset seeding. |
| `orbit-cmd` | CLI-facing command layer (doctor, migrate, diagnostics, templates, hooks) over `OrbitRuntime`. |
| `orbit-cli` | Clap-based CLI entrypoint. |

## Feature Ownership

Feature design docs live under `docs/design/<feature>/` and follow `docs/design/CONVENTIONS.md`.

| Feature | Folder | Lead |
|---------|--------|------|
| Knowledge graph | `knowledge-graph/` | `claude` |
| Policy & Sandboxing | `policy-sandbox/` | `claude` |
| Activity / Job | `activity-job/` | `codex` |
| Auditability | `auditability/` | `codex` |
| Groundhog | `groundhog/` | `codex` |
| User Interface | `user-interface/` | `gemini` |

## Design Mirror

The architecture design docs under `architecture/design/` are generated from the repository's `docs/design/` tree before `dev`, `check`, and `build`.

Do not edit generated mirror pages directly. Edit the source documents in `docs/design/`.
