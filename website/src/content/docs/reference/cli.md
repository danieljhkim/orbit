---
title: CLI Commands
description: "A compact map of the Orbit CLI command surface."
sidebar:
  order: 2
---

## Environment

| Command | Purpose |
|---------|---------|
| `orbit init` | Initialize the global Orbit root, skills, and skill links. |
| `orbit workspace` | Initialize and manage workspaces. |
| `orbit config` | Show or update Orbit configuration. |

## Operate

| Command | Purpose |
|---------|---------|
| `orbit run job <job_id>` | Run an arbitrary job by ID. |
| `orbit run ship [task_id ...]` | Submit backlog or explicitly selected tasks through the gated shipment pipeline and return a run ID immediately. |
| `orbit run ship --mode local [task_id ...]` | Run the local-only task path for backlog or explicitly selected tasks. |
| `orbit run duel-plan <task_id>` | Submit a planning duel for one task and return a run ID immediately. |
| `orbit run duel-plan <task_id> --wait` | Submit a planning duel and wait for its terminal status before returning. |
| `orbit task` | Create, update, and manage tasks. |
| `orbit task artifact put <task_id> <source_path>` | Store a UTF-8 file under a task's artifacts directory. |

## Observe

| Command | Purpose |
|---------|---------|
| `orbit graph` | Build and query the knowledge graph. See [Knowledge Graph](../concepts/knowledge-graph). |
| `orbit audit` | Query the audit event log. |
| `orbit run history` | Show recent job runs. Filter to one job with `-j <job_id>`. |
| `orbit run show [run_id]` | Show structured state and step summary for a job run (defaults to latest). |
| `orbit run logs [run_id]` | Print raw stdout/stderr captured for a job run. |
| `orbit run events [run_id]` | Show audit events recorded for a job run. |
| `orbit run trace [run_id]` | Show audit event parent/child trace for a job run. |

## Definitions

| Command | Purpose |
|---------|---------|
| `orbit activity` | View activity definitions. See [Activities & Jobs](../concepts/activities-jobs). |
| `orbit job` | View and manage job definitions. See [Activities & Jobs](../concepts/activities-jobs). |
| `orbit tool` | View and manage tools and external MCP plugins. |
| `orbit policy` | View filesystem profile policies and runtime scoping. See [Scoping](./scoping) and [Policy Format](./policy-format). |
| `orbit executor` | View executors. |

## Services

| Command | Purpose |
|---------|---------|
| `orbit mcp init` / `orbit mcp remove` | Register or unregister MCP client integration for Claude Code, Codex, Gemini, and Grok Build. |
| `orbit mcp serve` | Serve the safe default MCP tool surface. |
| `orbit web serve` | Serve the Orbit dashboard. |
