## Context
`orbit-cli` imported `orbit-knowledge` directly for graph build/update, show/search, history payloads, workspace-init graph build, and `.orbitignore` defaults. That made clap command files a second graph application layer and duplicated JSON shaping already present in the agent tool surface.

## Decision
Move those graph use cases into `orbit-tools::graph`, re-export them from `orbit-core::command::graph`, and keep `orbit-cli` on clap parsing plus human output. `orbit-core` continues to avoid a direct `orbit-knowledge` dependency; `orbit-tools` remains the only upstream graph consumer.

## Consequences
- `orbit-cli` no longer declares or imports `orbit-knowledge`.
- CLI and agent graph surfaces share the JSON payload builders for `show` and `history`.
- Workspace init still seeds `.orbitignore` and attempts the initial graph build through explicit core helpers.
- Cost: `orbit-tools` now contains a user-facing graph facade in addition to registered tools, so future maintainers must distinguish reusable use-case helpers from tool schemas and avoid accidentally registering CLI-only build/update behavior as agent tools.

---
