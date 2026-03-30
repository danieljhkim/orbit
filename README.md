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

# Initialize `~/.orbit` directory with default global configurations.
orbit init

# cd into your repo 
cd <repo>

# Initialize orbit workspace - creates `.orbit` dir in the repo_root
orbit workspace init

# Prompt an agent to create a task:
"Create an orbit task for ..."

# once task is created, approve the task
orbit task approve <task_id>

# run the parallel task pipeline
orbit job run job_parallel_task_pipeline
```

---

## Overview

### Orbit Artifacts

Orbit operates through a structured filesystem hierarchy under `.orbit/`:

```
.orbit/
├── activities/       # Atomic units of work (YAML)
├── diagnostics/      # Runtime diagnostics and health checks
├── jobs/
│   ├── jobs/         # Job definitions — ordered chains of activities
│   └── runs/         # Immutable execution audit logs per job run
├── scoreboard/       # Agent performance tracking (PR merge rates, friction bounty)
├── skills/           # Markdown-based skill instructions loaded by agents
└── tasks/            # Task artifacts organized by lifecycle state
```

### Tasks

Tasks are work items for agent/human coordination and project tracking. You can think of them as jira tickets.
- **Work unit for execution**: Represents a discrete piece of work (feature, bug fix, chore, refactor)
- **Lifecycle states**: Proposed → Backlog → InProgress → Review → Done (with branching to Blocked, Archived, Rejected, Someday)
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
- Example activities: run_tests, review_batch_pr, review_tasks, implement_change

### Jobs

You can chain one or more **activities** and run them as a single job.
- Jobs are workflows/pipelines that orchestrate activities. 
- Designed for automation/repeatability

```yaml
schemaVersion: 1
job:
  job_id: job_parallel_task_pipeline
  state: enabled
  max_active_runs: 1
  default_input:
    base: agent-main
    parallelism: 2
  steps:
    - target_type: activity
      target_id: dispatch_and_plan_batch
      agent_cli: claude
      model: opus
      condition: always
      timeout_seconds: 2000
    - target_type: activity
      target_id: snapshot_batch_state
      condition: on_success
      timeout_seconds: 30
    - target_type: activity
      target_id: parallel_dispatch_tasks
      condition: on_success
      timeout_seconds: 7200
    - target_type: activity
      target_id: verify_batch
      condition: on_success
      timeout_seconds: 600
    - target_type: activity
      target_id: commit_and_open_batch_pr
      condition: on_success
      timeout_seconds: 300
    - target_type: job
      target_id: job_batch_review_cycle
      condition: on_success
      timeout_seconds: 7200
```

---

## Architecture

Orbit is structured as a layered set of Rust crates. Lower layers have no knowledge of higher layers.

```
orbit-types          (leaf — shared types, OrbitError, ID generation)
    ↑
orbit-policy         (RBAC policy evaluation)
orbit-exec           (process spawning, sandboxing, timeouts)
    ↑
orbit-tools          (builtin tool registry: fs, git, github, orbit, proc, time, net)
    ↑
orbit-store          (file-based YAML + SQLite persistence, layered store)
orbit-agent          (agent provider abstraction: Claude, Codex, mock)
    ↑
orbit-engine         (activity/job execution, template rendering, retry logic)
    ↑
orbit-core           (runtime bootstrap, config layering, command dispatch, asset seeding)
    ↑
orbit-cli            (CLI entry point, clap-based commands, JSON/table output)
```

### Model Strategy

Orbit uses a multi-model strategy to balance reasoning depth against throughput:

| Model | Role | Rationale |
| :--- | :--- | :--- |
| **Claude (Opus)** | Planning, dispatch, review | High-order reasoning; architectural and code review quality |
| **Claude (Sonnet)** | Task dispatch, lightweight review | Fast reasoning for routing and triage |
| **Codex** | Implementation, code generation | High throughput and rate limits for iterative coding tasks |

---

## Store & Scoping Model

Orbit maintains two `.orbit/` directories: a **global** root (`~/.orbit/`) and a **workspace** root (`.orbit/` inside a repo). Each resource type has a fixed merge strategy:

| Resource | Strategy | Behavior |
| :--- | :--- | :--- |
| Tasks, Runs, Scoreboard | **WorkspaceOnly** | Read/write only in the workspace `.orbit/` |
| Audits, Configs, Workspace registry | **GlobalOnly** | Read/write only in `~/.orbit/` |
| Activities, Jobs | **MergeByKey** | Global defaults + workspace overrides; workspace entries shadow global by key |
| Skills | **WorkspaceReplaces** | Workspace has full control over available skills |

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
