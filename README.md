# Orbit: Local-First Agentic Workflow Engine

Orbit is a lightweight, local-first execution engine designed for agent-driven software delivery. It provides the core primitives necessary to orchestrate complex, multi-step AI workflows directly within a development environment.

Unlike centralized AI orchestration platforms, Orbit is built to be a decoupled execution layer. It focuses on the "how" of agentic task completion, leaving scheduling and higher-level management to external tools or manual triggers.

---

## Quick Start

**1. Initialize Orbit in your repository:**

```bash

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

### Task Lifecycle

Tasks move through a linear lifecycle with defined gates at each transition:

```
proposed → backlog → in-progress → review → done
```

Rejection and recovery paths:

```
proposed  → rejected
review    → rejected
rejected  → backlog   (reconsider)
```

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


### Architecture

Orbit is structured as a layered set of Rust crates. Lower layers have no knowledge of higher layers.

```
orbit-cli          ← binary; CLI parsing and command dispatch
    └── orbit-core         ← runtime facade; wires all subsystems together
            ├── orbit-engine       ← job and activity execution loop
            │       ├── orbit-agent    ← agent invocation (Claude, Codex)
            │       ├── orbit-exec     ← subprocess execution and sandboxing
            │       ├── orbit-tools    ← built-in tool registry
            │       └── orbit-store    ← persistence backends
            ├── orbit-policy       ← tool access control and role enforcement
            └── orbit-types        ← shared domain types and error definitions
```


### Model Strategy

Orbit uses a dual-model strategy to balance reasoning depth against throughput:

| Model | Role | Rationale |
| :--- | :--- | :--- |
| **Claude** | Planning, dispatch, review | High-order reasoning; architectural and code review quality |
| **Codex** | Implementation, code generation | High throughput and rate limits for iterative coding tasks |

---

## Current Status

Orbit is in active development (WIP). Core execution primitives are stable for local use; production deployments are not yet recommended.

### Persistence Note

All Orbit state lives in `.orbit/` and is local by default. For persistence across machines without polluting git history, consider a Git overlay tool such as **monodev**. Orbit state overlays are tracked here:

- https://github.com/danieljhkim/orbit/tree/monodev/persist/persist/stores/orbit-states/overlay/.orbit

---

## Contributing

Contributions focused on core execution primitives, state serialization, or tool-calling interfaces are welcome. Open an issue or submit a pull request for review.
