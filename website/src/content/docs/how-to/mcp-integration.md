---
title: Set Up MCP
description: "Expose Orbit's safe MCP tool surface to Claude Code, Codex, Gemini, or Grok Build."
sidebar:
  order: 5
---

## Initialize

Use auto-detection:

```bash
orbit mcp init --auto
```

Or target a client explicitly:

```bash
orbit mcp init --claude
orbit mcp init --codex
orbit mcp init --gemini
orbit mcp init --grok
```

**Grok Build** uses the native `.grok/config.toml` format (similar to how Claude Code can use a config file). `orbit mcp init --grok` will create or update `.grok/config.toml` in your workspace root (or `~/.grok/config.toml` for global).

## Serve

Start the MCP surface:

```bash
orbit mcp serve
```

Use `orbit tool list` to inspect the current local registry. MCP exposure is a
capability-filtered subset of that registry. The retired graph tools are not
exposed.

## Listen on a socket

Deployments that need the server on a port — a server-side Orbit reached through
an SSH tunnel, for example — use the listener instead:

```bash
orbit mcp listen              # binds 127.0.0.1:7879
orbit mcp listen 127.0.0.1:9000
```

It serves the same tool surface as `orbit mcp serve`, one independent session per
connection. The socket authenticates no client, so it binds loopback; a wider bind
requires `--allow-non-loopback` and a network path you have restricted by other
means.

## Remove

```bash
orbit mcp remove --all
```
