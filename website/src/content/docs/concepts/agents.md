---
title: Agents
description: "How Orbit invokes coding agents in v1 — CLI subprocesses under a supervised runtime."
sidebar:
  order: 6
---

## Runtime Paths

**v1 ships CLI backends only.** Orbit invokes coding agents by spawning their official CLIs (Codex, Claude Code, Gemini CLI, etc.) as supervised subprocesses under an `FsProfile` and policy guardrails. The agent CLI is responsible for talking to its provider; Orbit does not need a separate provider API key for the v1 path.

| Backend | v1 status | Role |
|---------|-----------|------|
| `cli`   | Supported | Subprocess-backed provider CLIs. The only release-supported invocation path in v1. |
| `http`  | Preview / not in v1 release surface | Programmatic provider communication via `LoopTransport`. Wired in code and exercised in tests, but not covered by the v1 release contract. Slated to become primary in v2. |

`backend: auto` resolves before dispatch and folds to the configured default. Downstream execution always sees a concrete backend.

## Providers

Schema v2 provider values include:

- `claude`
- `codex`
- `gemini`

In v1 each provider runs through its CLI runtime under `orbit-agent::providers/<name>/`. HTTP transport support is provider-specific and not part of the v1 release surface; an HTTP attempt against an unwired provider fails structurally rather than silently falling back to CLI.

## Tool Allowlists

Agent-loop activities declare the tool names an agent may call. Empty means no tools are allowed.

```yaml
spec:
  type: agent_loop
  tools:
    - orbit.task.show
    - orbit.graph.search
```

`on_denial` controls whether a denied tool call terminates the loop or returns a structured error for the agent to handle. In v1 (CLI backend) the agent CLI executes inside a supervised subprocess; tool allowlist enforcement is delegated to the harness and recorded as a `tool_allowlist.harness_delegated` envelope event in the audit trail.

## Platform Support

Bundled agent executors (`claude`, `codex`, `gemini`) declare `sandbox: macos-sandbox-exec`, so the spawned subprocess is wrapped in macOS `sandbox-exec` with the activity's resolved `FsProfile` compiled to SBPL. **This OS-level isolation is macOS only.** On Linux and Windows, `EnvironmentHost::resolve_executor_sandbox` rejects the platform mismatch — Orbit's process supervision and tool allowlist still apply, but the agent subprocess itself runs without a kernel-level sandbox. The bundled `local-shell` executor has no sandbox declaration on any platform by design.
