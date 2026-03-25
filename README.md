# Orbit: Local-First Agentic Workflow Engine

Orbit is a lightweight, local-first execution engine designed for agent-driven software delivery. It provides the core primitives necessary to orchestrate complex, multi-step AI workflows directly within a development environment.

Orbit runs directly on agent CLIs - no API keys required.

---

## Quick Start

**Prerequisites**: Rust + Codex CLI / Claude Code (subscriptions required)

Example flow:

```bash
# install orbit binary
make install

# Initialize `.orbit/` directory with default configuration.
orbit init

# Prompt an agent to create a task:
"Create this orbit task ...."

# once task is created (proposed status), approve the task
orbit task approve <task_id>

# run job_task_pipeline job
orbit job run job_task_pipeline 
```

---

## Overview

### Orbit Artifacts

Orbit operates through a structured filesystem hierarchy under `.orbit/`:

```
.orbit/
├── activities/       # Atomic units of work (YAML)
├── jobs/
│   ├── jobs/         # Job definitions — ordered chains of activities
│   └── runs/         # Immutable execution audit logs per job run
├── identities/       # Agent personas with roles and behavioral profiles
├── skills/           # Markdown-based skill instructions loaded by agents
├── tasks/            # Task artifacts organized by lifecycle state
└── orbit.db          # SQLite audit store
```

### Tasks

Tasks are work items for agent/human coordination and project tracking. You can think of them as jira tickets.
- **Work unit for execution**: Represents a discrete piece of work (feature, bug fix, chore, refactor)
- **Lifecycle states**: Proposed → Backlog → InProgress → Review → Done (with branching to Blocked, Archived, Rejected)
- **Execution tracking**:
    - execution_summary for recording what was actually done
    - Branch and PR number for code changes
    - Comments from team members
    - History log of all transitions and updates

### Activities

Activities are atomic, reusable units of work:

- Self-contained operations with defined schemas (input/output)
- Each has a spec_type that determines how it's implemented (i.e. automation, agent_invoke, api, cli_command)
- Can be run individually via CLI or as part of a job
- Example activities: run_tests, review_pr, review_tasks, implement_change

### Jobs

You can chain one or more **activities** and run them as a single job.
- Jobs are workflows/pipelines that orchestrate activities. 
- Designed for automation/repeatability

```yaml
schemaVersion: 1
job:
  job_id: job_task_pipeline
  state: enabled
  default_input:
    base: agent-main
  steps:
    - target_type: activity
      target_id: dispatch_task
      agent_cli: claude
      timeout_seconds: 1000
      env_extra: []
    - target_type: activity
      target_id: create_branch
      agent_cli: ""
      timeout_seconds: 60
      env_extra: []
    - target_type: activity
      target_id: implement_change
      agent_cli: codex
      timeout_seconds: 1800
      env_extra: []
    - target_type: activity
      target_id: run_tests
      agent_cli: ""
      timeout_seconds: 600
      env_extra: []
    - target_type: activity
      target_id: commit_changes
      agent_cli: ""
      timeout_seconds: 60
      env_extra: []
    - target_type: activity
      target_id: open_pr
      agent_cli: ""
      timeout_seconds: 300
      env_extra: []
    - target_type: activity
      target_id: review_pr
      agent_cli: claude
      timeout_seconds: 600
      env_extra: []
```

---

## Architecture

Orbit is structured as a layered set of Rust crates. Lower layers have no knowledge of higher layers.

```
                        ┌─────────────────────────────────────────┐
                        │            orbit-types (leaf)            │
                        └──┬──────┬──────┬──────┬─────────────────┘
                           │      │      │      │
                       policy   exec   store   agent
                           │      │
                           └──┬───┘
                            tools
                              │
                        ┌─────┴─────┐
                        │  engine   │ → store, agent, exec, tools
                        └─────┬─────┘
                        ┌─────┴─────┐
                        │   core    │ → engine, store, agent, policy, exec, tools
                        └─────┬─────┘
                        ┌─────┴─────┐
                        │    cli    │
                        └───────────┘
```


### Model Strategy

Orbit uses a dual-model strategy to balance reasoning depth against throughput:

| Model | Role | Rationale |
| :--- | :--- | :--- |
| **Claude** | Planning, dispatch, review | High-order reasoning; architectural and code review quality |
| **Codex** | Implementation, code generation | High throughput and rate limits for iterative coding tasks |

---

## Store & Scoping Model

Orbit maintains two `.orbit/` directories: a **global** root (`~/.orbit/`) and a **workspace** root (`.orbit/` inside a repo). Each resource type has a fixed merge strategy:

| Resource | Strategy | Behavior |
| :--- | :--- | :--- |
| Tasks, Runs, Scoreboard | **Workspace-only** | Read/write only in the workspace `.orbit/` |
| Audits, Configs, Workspace registry | **Global-only** | Read/write only in `~/.orbit/` |
| Activities, Jobs, Skills, Tools | **MergeByKey** | Global defaults + workspace overrides; workspace entries shadow global by key |

When workspace and global roots are the same directory (e.g. `orbit init` in `$HOME`), layering is a no-op — a single store is used.

These strategies are hardcoded per resource type; there is no runtime configuration.

---

## Current Status

Orbit is in active development (WIP). Core execution primitives are stable for local use; production deployments are not yet recommended.

### Persistence Note

All Orbit state lives in `.orbit/` and is local by default. For persistence across machines without polluting git history, consider a Git overlay tool such as **monodev**. Orbit state overlays are tracked here:

- https://github.com/danieljhkim/orbit/tree/monodev/persist/persist/stores/orbit-states/overlay/.orbit

---

## Contributing

Contributions focused on core execution primitives, state serialization, or tool-calling interfaces are welcome. Open an issue or submit a pull request for review.
