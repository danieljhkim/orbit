---
title: Set Up MCP
description: "Expose Orbit's safe MCP tool surface to Claude, Codex, or Gemini."
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
