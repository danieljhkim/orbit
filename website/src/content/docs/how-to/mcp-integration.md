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

Start the default safe MCP surface:

```bash
orbit mcp serve
```

The default surface includes task tools and graph read tools. Experimental graph write tools require explicit opt-in:

```bash
orbit mcp serve --allow-write
```

## Remove

```bash
orbit mcp remove --all
```
