---
title: Crate Layout
description: "The Orbit Rust workspace layout and dependency direction."
sidebar:
  order: 3
---

## Crates

| Crate | Responsibility |
|-------|----------------|
| `orbit-cli` | Clap-based CLI entrypoint. |
| `orbit-cmd` | CLI-facing command layer extracted from `orbit-core` (ORB-10016): doctor, migrate, diagnostics, task templates, hooks, agent-rules, direct v2 activity runs. Exposes `*Commands` extension traits over `OrbitRuntime`. |
| `orbit-core` | Runtime bootstrap, config layering, default asset seeding, and the runtime-integrated command modules. Surfaces `OrbitRuntime` to `orbit-cmd`, `orbit-cli`, and `orbit-dashboard`. Does **not** depend on `orbit-agent` or `orbit-cmd`. |
| `orbit-engine` | Activity and job execution, template rendering, retry logic. Owns the `backend: cli` subprocess runner, which references `orbit-agent::{Agent, AgentConfig}` directly. |
| `orbit-agent` | Per-provider `AgentRuntime` implementations under `providers/<name>/<name>_runtime.rs` (claude, codex, gemini, openai_compat, anthropic, ollama, mock_agent). Hosts HTTP `LoopTransport` primitives. |
| `orbit-tools` | Tool registry plus built-in graph, fs, and policy-aware exec tools. |
| `orbit-knowledge` | Knowledge/graph parsing and storage helpers. Multi-language source parsing (Rust, Go, Java, JavaScript, Python). |
| `orbit-policy` | Filesystem-scoping policy engine. Owns `FsProfile` resolution and `denyRead` / `denyModify` evaluation. |
| `orbit-exec` | Process / sandbox / supervision primitives for shell-command execution under an `FsProfile`. |
| `orbit-store` | Layered store pattern (YAML + SQLite). |
| `orbit-mcp` | Model Context Protocol adapter using `rmcp`. Consumed by `orbit-cli` via `orbit mcp serve`. |
| `orbit-common` | Leaf — shared domain types (`OrbitError`, IDs, activity/job schemas) and generic utilities (fs, redaction, logging, blob storage). |

## Dependency Direction

```
orbit-common → orbit-policy, orbit-exec, orbit-knowledge → orbit-tools → orbit-agent → orbit-engine → orbit-core → orbit-cmd → orbit-cli
            ↘ orbit-store ──────────────────────────────────────────────────↗            ↗
            ↘ orbit-mcp ─────────────────────────────────────────────────────────────────────────────────────────↗
```

Do not add cross-crate dependencies that violate this direction. Lower layers stay reusable and free of higher-level runtime concerns. In particular, `orbit-core` must not depend on `orbit-agent` (the `backend: cli` subprocess runner in `orbit-engine` is the bridge) and must never depend on `orbit-cmd` (the extracted command layer depends on the runtime, not the other way around).
