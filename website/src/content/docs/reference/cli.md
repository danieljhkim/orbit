---
title: CLI Commands
description: "A compact map of the Orbit CLI command surface."
sidebar:
  order: 2
---

## Environment

| Command | Purpose |
|---------|---------|
| `orbit init` | Initialize the global Orbit root. |
| `orbit workspace` | Initialize and manage workspaces. |
| `orbit config` | Show or update Orbit configuration. |

## Operate

| Command | Purpose |
|---------|---------|
| `orbit run <job_id>` | Run a job workflow. Direct shorthand for `orbit run job <job_id>`. |
| `orbit run job <job_id>` | Run an arbitrary job by ID. |
| `orbit run ship <task_id> ...` | Ship explicitly selected tasks through the PR pipeline by default. |
| `orbit run ship --mode local <task_id> ...` | Run the local-only task path for explicitly selected tasks. |
| `orbit run ship-auto` | Auto-select backlog tasks and ship them through the task pipeline. |
| `orbit run duel-plan <task_id>` | Run a planning duel for one task. |
| `orbit task` | Create, update, and manage tasks. |

## Observe

| Command | Purpose |
|---------|---------|
| `orbit graph` | Build and query the knowledge graph. |
| `orbit audit` | Query the audit event log. |
| `orbit metrics` | Inspect token, tool-call, and knowledge-pack metrics. |
| `orbit scoreboard` | Generate read-only scoreboard summaries. |
| `orbit job history <job_id>` | Inspect job run history, including workflow runs. |
| `orbit job run-state <run_id>` | Inspect persisted state for a job run. |

## Definitions

| Command | Purpose |
|---------|---------|
| `orbit activity` | View activity definitions. |
| `orbit job` | View and manage job definitions. |
| `orbit tool` | View and manage tools and external MCP plugins. |
| `orbit policy` | View filesystem profile policies and runtime scoping. |
| `orbit executor` | View executors. |

## Services

| Command | Purpose |
|---------|---------|
| `orbit mcp init` / `orbit mcp remove` | Register or unregister MCP client integration for Claude, Codex, and Gemini. |
| `orbit mcp serve` | Serve the safe default MCP tool surface. |
| `orbit web serve` | Serve the Orbit dashboard. |
