---
title: CLI Commands
description: "A compact map of the Orbit CLI command surface."
sidebar:
  order: 2
---

## Setup

| Command | Purpose |
|---------|---------|
| `orbit init` | Initialize the global Orbit root. |
| `orbit workspace` | Initialize and manage workspaces. |
| `orbit mcp` | Manage MCP client integrations. |
| `orbit config` | Show or update Orbit configuration. |

## Resources

| Command | Purpose |
|---------|---------|
| `orbit task` | Create, update, and manage tasks. |
| `orbit activity` | Define, list, and run activities. |
| `orbit job` | Define, list, and manage job workflows. |
| `orbit policy` | Manage filesystem profile policies and runtime scoping. |
| `orbit executor` | Manage executors. |
| `orbit tool` | Manage tools and external MCP plugins. |

## Workflows

| Command | Purpose |
|---------|---------|
| `orbit run ship <task_id> ...` | Ship explicitly selected tasks through the PR pipeline by default. |
| `orbit run ship --mode local <task_id> ...` | Run the local-only task path for explicitly selected tasks. |
| `orbit run ship-auto` | Auto-select backlog tasks and ship them through the task pipeline. |
| `orbit run duel-plan <task_id>` | Run a planning duel for one task. |
| `orbit run job <job_id>` | Run an arbitrary job by ID. |
| `orbit run <job_id>` | Direct shorthand for `orbit run job <job_id>`. |
| `orbit job history <job_id>` | Inspect job run history, including workflow runs. |
| `orbit job run-state <run_id>` | Inspect persisted state for a job run. |

## Inspect

| Command | Purpose |
|---------|---------|
| `orbit audit` | Query the audit event log. |
| `orbit metrics` | Inspect token, tool-call, and knowledge-pack metrics. |
| `orbit scoreboard` | Generate read-only scoreboard summaries. |
| `orbit graph` | Build and query the knowledge graph. |

## Serve

| Command | Purpose |
|---------|---------|
| `orbit serve mcp` | Serve the safe default MCP tool surface. |
| `orbit serve web` | Serve Orbit outward when the web surface is enabled. |
