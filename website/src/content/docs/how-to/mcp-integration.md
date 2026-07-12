---
title: Set Up MCP
description: "Expose Orbit's safe MCP tool surface to Claude Code, Codex, Gemini, or Grok Build."
sidebar:
  order: 5
---

## Agent plugins

The official Claude Code and Codex plugins register the MCP server and shared
Orbit skills automatically. They pull the native binary through the
`@orbit-tools/cli` npm proxy on the first MCP call, so the `orbit` CLI does not
need to be installed separately.

### Claude Code

```text
/plugin marketplace add danieljhkim/orbit
/plugin install orbit
```

To update later, run `/plugin update orbit` and restart Claude Code.

### Codex

```bash
codex plugin marketplace add danieljhkim/orbit --ref main
codex plugin add orbit@orbit
```

To update later, refresh the Git marketplace snapshot and reinstall the plugin:

```bash
codex plugin marketplace upgrade orbit
codex plugin add orbit@orbit
```

Both plugin paths require Node 18+ on `PATH`. Skip the manual registration flow
below when you use a plugin.

### Verify a plugin installation

Start a fresh task from an initialized Orbit workspace. These prompts ask the
agent to discover the bundled Orbit skill and invoke the read-only
`orbit.task.list` MCP tool:

```bash
# Claude Code
claude -p 'Use the read-only orbit.task.list MCP tool to list up to three tasks, then report how many were returned.'

# Codex CLI
codex -C <repo> 'Use the read-only orbit.task.list MCP tool to list up to three tasks, then report how many were returned.'
```

For Codex, `codex mcp list` should also show an `orbit` server whose command is
the canonical `npx -y @orbit-tools/cli@latest mcp serve` transport declared by
the plugin manifest.

## Initialize (manual)

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

The surface includes task tools and graph read tools. Graph write tools are not exposed; write coordination is handled through task lock reservations before dispatch.

## Remove

```bash
orbit mcp remove --all
```

<!-- ORB-10117 -->
