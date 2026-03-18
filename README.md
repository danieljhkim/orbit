# Orbit: Local-First Agentic Workflow Engine

Orbit is a lightweight, local-first execution engine designed for agent-driven software delivery. It provides the core primitives necessary to orchestrate complex, multi-step AI workflows directly within a development environment.

Unlike centralized AI orchestration platforms, Orbit is built to be a decoupled execution layer. It focuses on the "how" of agentic task completion, leaving scheduling and higher-level management to external tools or manual triggers.

## Core Architectural Principles

* **Decoupled Execution:** Orbit functions as a pure execution engine. It manages task state, tool-calling, and artifact generation without enforcing a specific scheduling or hosting model.
* **Local-First Persistence:** All engine state, including job definitions, run audits, and task tracking, is stored within a local `.orbit` directory. This ensures high-speed execution and repository-resident context.
* **Multi-Model Orchestration:** Orbit optimizes for both reasoning quality and execution throughput by assigning specialized roles to different Large Language Models (LLMs).

## System Components

The engine operates through a structured filesystem hierarchy within the `.orbit` directory:

* **Activities (`.orbit/activities`):** Definitions of atomic units of work that can be performed by an agent/automation/API/script.
* **Jobs (`.orbit/jobs/jobs`):** Executable chains of activities that link multiple activities into a cohesive delivery pipeline.
* **Runs (`.orbit/jobs/runs`):** Immutable execution audits and artifacts. This serves as the "source of truth" for debugging and state rehydration.
* **Skills (`.orbit/skills`):** A library of orbit-realted capabilities that extend the agent's ability to utilize orbit workflow.
* **Tasks (`.orbit/tasks`):** Local task artifacts that manage the lifecycle of a feature or bug fix, similar to Jira board.

## Model Dynamics and Strategy

Orbit utilizes a dual-model strategy to balance computational cost against reasoning depth:

| Model | Primary Role | Justification |
| :--- | :--- | :--- |
| **Claude** | Planning and Review | High-order reasoning and adherence to complex architectural patterns. |
| **Codex** | Code Generation and Execution | High throughput and generous rate limits for iterative implementation tasks. |

## Current Status

Orbit is currently in an active development (WIP) phase. While the core execution primitives are stable enough for local productivity enhancements, it is not yet recommended for production environments.

### Implementation Note

To maintain repository hygiene, it is recommended to add the `.orbit/` directory to your `.gitignore`. For users who wish to persist these artifacts across machines without polluting the main Git history, consider using a Git overlay tool such as **monodev**. For Orbit, .orbit overlays are tracked here:

- https://github.com/danieljhkim/orbit/tree/monodev/persist/persist/stores/orbit-states/overlay/.orbit

## Contributing

Contributions focusing on refining the core execution primitives, improving state serialization, or hardening tool-calling interfaces are welcome. Please open an issue or submit a pull request for review.