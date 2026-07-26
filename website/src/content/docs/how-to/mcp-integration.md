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

## Remove

```bash
orbit mcp remove --all
```
