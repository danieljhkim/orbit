---
title: Set Up MCP
description: "Expose Orbit's safe MCP tool surface to Claude, Codex, or Gemini."
sidebar:
  order: 5
---

## Claude Code (plugin path)

For Claude Code, the simplest setup is the official plugin — it registers the MCP server, skills, and subagents in one step and pulls the native binary via the `@orbit-dev/cli` npm proxy:

```text
/plugin marketplace add danieljhkim/orbit
/plugin install orbit
```

Requires Node 18+ on `PATH`. Skip the rest of this page if you go this route; the plugin handles registration. Use the manual flow below for Codex, Gemini, or a Claude Code install you want to wire by hand.

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
```

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
