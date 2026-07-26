---
title: Architecture
description: "Orbit's crate boundaries and dependency direction."
sidebar:
  order: 1
---

## Crate Graph

Orbit is a layered Rust workspace. Lower layers do not depend on higher layers.

```mermaid
flowchart LR
  CLI["orbit-cli"] --> Cmd["orbit-cmd"]
  CLI --> Core["orbit-core"]
  CLI --> Remote["orbit-remote"]
  CLI --> Dashboard["orbit-dashboard"]
  Dashboard --> Core
  Dashboard --> Remote
  Dashboard --> Cmd
  Cmd --> Core
  Cmd --> Engine
  Cmd --> Store
  Core --> Engine["orbit-engine"]
  Core --> Store["orbit-store"]
  Core --> Tools["orbit-tools"]
  Core --> Search["orbit-search"]
  Core --> Policy["orbit-policy"]
  Engine --> Agent["orbit-agent"]
  Engine --> Exec["orbit-exec"]
  Engine --> Store
  Engine --> Tools
  Agent --> Tools
  Agent --> Common["orbit-common"]
  Tools --> Exec
  Tools --> Policy
  Remote --> Core
  Remote --> Store
  Remote --> Tools
  Remote --> MCP["orbit-mcp"]
  Remote --> Common
  Graph --> Common
  Search --> Common
  MCP --> Common
  Store --> Common
  Exec --> Common
  Policy --> Common
  Tools --> Common
  Core --> Common
  Cmd --> Common
```

Arrows point from a consumer to its dependency. `orbit-store` and `orbit-mcp` are
neutral kernels that depend only on `orbit-common`. The vertical `orbit-remote`
feature composes registry persistence, MCP policy, broker/hub routing, SSH links,
and spoke registration without introducing a reverse dependency from those kernels.
Layering constrains dependency direction, not feature ownership: a vertical crate
may own its domain model, feature schema, transport policy, and composition end to
end while reusing neutral mechanisms.
`orbit-core` does **not** depend on `orbit-agent`; the bridge is `orbit-engine`'s
`backend: cli` subprocess runner.

## Boundaries

| Crate | Role |
|-------|------|
| `orbit-common` | Shared domain types, errors, IDs, utility helpers. |
| `orbit-policy` | Filesystem-scoping policy and profile resolution. |
| `orbit-exec` | Process, sandbox, and supervision primitives. |
| `orbit-store` | Generic YAML/SQLite stores, connection primitives, namespaced feature-migration ledger, and immutable historical bootstrap migrations. |
| `orbit-graph` | Parked, dependent-free derived graph index and query library with no CLI, MCP, or tool-registry surface. |
| `orbit-search` | Retrieval/ranking feature and workspace-local semantic index; also builds `orbit-search-companion`, a separately installed embedding companion binary, as an additional `[[bin]]` target. |
| `orbit-agent` | HTTP loop transport and retained CLI runtimes. |
| `orbit-engine` | Activity/job execution, template rendering, retries, CLI subprocess runner. |
| `orbit-tools` | Generic built-in tool registry and external tool integration. |
| `orbit-mcp` | Generic RMCP framing, server composition, and raw-client kernel. |
| `orbit-remote` | Vertical host/workspace registry, feature persistence, MCP contract and extensions, broker, hub, SSH link, and registration composition. |
| `orbit-core` | Neutral runtime bootstrap, config, runtime-integrated commands, coordination executor, and default asset seeding. |
| `orbit-cmd` | CLI-facing command layer (doctor, migrate, diagnostics, templates, hooks) over `OrbitRuntime`. |
| `orbit-dashboard` | Web dashboard and HTTP API over Core and Remote registry state. |
| `orbit-cli` | Clap-based entrypoint and local client-configuration surface; delegates Remote behavior to `orbit-remote`. |

Detailed implementation records remain alongside the source code. They are not
mirrored into this public reference because they contain historical interfaces
and repository-internal artifact references.
