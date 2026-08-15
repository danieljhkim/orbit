---
title: Agents
description: "How Orbit invokes coding agents through CLI and HTTP runtimes."
sidebar:
  order: 6
---

## Runtime Paths

Orbit spawns official provider CLIs as supervised subprocesses under an
`FsProfile` and policy guardrails. The agent CLI is responsible for talking to
its provider.

This is the only agent execution path. The `backend: http | cli | auto`
selector was retired: an activity, job, or config that still declares
`backend: cli` keeps working and the value is ignored, while `http` and `auto`
are rejected with a migration message rather than being remapped onto the CLI
agent.

## Providers

Canonical provider values are:

- `claude`
- `codex`
- `gemini`
- `grok`
- `ollama`
- `openai_compat`

CLI execution is available for `claude`, `codex`, `gemini`, and `grok`.
Transport support is provider-specific; selecting an unsupported
provider fails instead of silently switching providers.

## Tool Allowlists

Agent-loop activities declare the tool names an agent may call. Empty means no tools are allowed.

```yaml
spec:
  type: agent_loop
  tools:
    - orbit.task.show
    - orbit.search
```

`on_denial` controls whether a denied tool call terminates the loop or returns a
structured error for the agent to handle. Under agent dispatch, tool allowlist
enforcement is delegated to the harness and recorded in the audit trail.

## Platform Support

Bundled agent executors (`claude`, `codex`, `gemini`, `grok`) declare
`sandbox: macos-sandbox-exec`, so the spawned subprocess is wrapped in macOS
`sandbox-exec` with the activity's resolved `FsProfile` compiled to SBPL.
**This OS-level isolation is macOS only.** On Linux and Windows, Orbit's process
supervision and tool allowlist still apply, but the agent subprocess itself runs
without a kernel-level sandbox. The bundled `local-shell` executor has no
sandbox declaration on any platform by design.
