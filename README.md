# Orbit: Local-First Agentic Workflow Engine

Orbit is a local-first workflow engine for agent-driven software delivery. It helps humans and coding agents coordinate around structured tasks, reusable activities, and repeatable job pipelines directly inside a repository.

Orbit runs on top of agent CLIs such as Codex and Claude Code. No provider API keys are required by Orbit itself.

## Philosophy

> Minimize tokens. Minimize irrelevant decisions. Maximize determinism.

Orbit should make agents spend judgment on what matters: intent, correctness, tradeoffs, design, and risk. Agent opinions and decisions should directly support these values; if a choice does not reduce token usage, remove irrelevant discretion, or make the system more deterministic, Orbit should make that choice instead.

---

## Quick Start

**Prerequisites**: Rust, Codex CLI, and Claude Code

For the PR-based workflow (`orbit run ship`), you also need the GitHub CLI (`gh`) installed and authenticated. If you do not want to use GitHub or open pull requests, use `orbit run ship-local` instead.

```bash
# install via Homebrew (macOS)
brew install danieljhkim/tap/orbit

# or build from source
git clone https://github.com/danieljhkim/orbit.git
cd orbit
make install

# initialize global Orbit state (~/.orbit)
orbit init

# enter a repository and initialize workspace-local Orbit state
cd <repo>
orbit workspace init

# ask an agent to create one or more Orbit tasks
"Create an orbit task for <task_description>"

# review and approve the proposed task as a human
orbit task list
orbit task show <task_id>
orbit task approve <task_id> --note "LGTM"

# run the default PR-based workflow (this requires gh)
orbit run ship

# or run a local-only workflow with no PR/review loop
orbit run ship-local
```

If you already know which tasks you want to run, pin them explicitly:

```bash
orbit run ship --tasks T123,T456 --parallelism 2 --base main
```

---

## Mental Model

Orbit has four core concepts:

- **Task**: a unit of work to be proposed, approved, implemented, reviewed, and tracked
- **Activity**: a reusable operation with a defined input/output shape
- **Job**: an ordered workflow made of activities and nested jobs
- **Workflow**: a human-friendly alias over a common job pipeline, exposed through `orbit run`

In practice:

- Humans review and approve work
- Agents create and implement tasks
- Orbit jobs orchestrate planning, execution, review, and merge

---

## Canonical Path

For most repositories, the intended path is:

1. Create tasks
2. Review and approve them
3. Run `orbit run ship`
4. Let Orbit dispatch agents, verify results, open a PR, review it, and merge when approved

Most users should start with the first-class workflows below. Reach for `orbit job run ...` only when you need lower-level control or custom automation behavior.

Note: `orbit run ship` depends on GitHub-backed PR operations and therefore requires `gh` to be installed and logged in. `orbit run ship-local` does not.

---

## Who Uses Which Surface?

Orbit has different surfaces for different actors:

- **Humans** usually use the CLI directly: `orbit task ...`, `orbit run ...`, `orbit job ...`
- **Agents** should use the Orbit tool surface: `orbit tool run ...`
- **Automations/jobs** operate through activities, runtime hooks, and registered tools

These surfaces are intentionally distinct because Orbit records provenance and execution context differently for humans, agents, and automation. If you are writing agent workflows, prefer the tool surface rather than shelling out to human-oriented CLI commands.

---

## First-Class Workflows

`orbit run` provides ergonomic aliases for the most common flows:

| Workflow | Command | Description |
| :--- | :--- | :--- |
| **ship** | `orbit run ship` | Select tasks, dispatch agents, verify results, open a PR, review, and merge; requires `gh` auth |
| **ship-local** | `orbit run ship-local` | Select tasks, dispatch agents, and commit locally without a PR |
| **review** | `orbit run review` | Review tasks in `proposed` or `review` state |

Optional flags for `ship` and `ship-local`:

- `--tasks T1,T2` pin specific task IDs instead of auto-selecting from backlog
- `--parallelism N` control the number of parallel workers
- `--base BRANCH` override the base branch

Examples:

```bash
# auto-select tasks from backlog
orbit run ship

# pin specific tasks
orbit run ship --tasks T20260402-0352,T20260402-0406 --parallelism 2

# local-only pipeline
orbit run ship-local --base main

# review queue
orbit run review
```

For advanced cases, the lower-level job interface remains available:

```bash
orbit job run <job_id> --input '{"base":"main"}'
```

---

## CLI Surface

Orbit exposes a small set of top-level command groups:

```text
orbit [OPTIONS] <COMMAND>

Run workflows:
  run        Run a first-class workflow
  job        Define and run automation jobs
  job-run    Inspect and manage job run history

Manage work:
  task       Create, update, and manage tasks
  activity   Manage activity definitions
  skill      Manage agent skill definitions
  tool       Manage and run Orbit tools

Configure and inspect:
  config     Show or update Orbit configuration
  init       Initialize the global Orbit root (~/.orbit)
  workspace  Initialize and manage workspaces
  audit      Query the audit event log
```

---

## Workspace Model

Orbit artifacts have two scopes:

- **Global scope**: initialized via `orbit init`, usually under `~/.orbit/`
- **Workspace scope**: initialized via `orbit workspace init`, under `<repo>/.orbit/`

Orbit operates through a structured hierarchy under `.orbit/`:

```text
.orbit/
├── activities/       # Activity definitions (YAML)
├── diagnostics/      # Runtime diagnostics and health checks
├── jobs/
│   ├── jobs/         # Job definitions
│   └── runs/         # Immutable execution logs per job run
├── scoreboard/       # Derived performance metrics and scoring artifacts
├── skills/           # Agent skill instructions
└── tasks/            # Task artifacts organized by lifecycle state
```

Scoping rules matter:

- Tasks, job runs, and scoreboards are workspace-local
- Activities and jobs merge from global defaults with workspace overrides
- Skills are fully controlled by the workspace
- Audit is global

---

## Tasks

Tasks are work items for agent and human coordination, similar to Jira tickets but designed for agent execution.

Tasks are tightly coupled to Orbit jobs. If you already use Linear or Jira, Orbit tasks can serve as the execution-layer counterpart inside the repo.

- **Work unit**: feature, bug fix, chore, refactor, or follow-up
- **Lifecycle**: Proposed → Backlog → In Progress → Review → Done
- **Side paths**: Blocked, Rejected, Archived, Someday
- **Tracked state**: acceptance criteria, plan, execution summary, PR metadata, comments, and history

---

## Activities And Jobs

Activities are atomic, reusable operations:

- They have defined schemas and input/output contracts
- They can be implemented as automation, agent invocation, or CLI commands
- They can run independently or as part of a job

Jobs are workflows composed of activities:

- They encode repeatable automation
- They can call nested jobs
- They are the engine behind the higher-level `orbit run` workflows

---

## Default Jobs

Orbit currently ships with a small set of default jobs that cover planning, implementation, review, and merge.

### `job_parallel_task_pipeline`

The main PR-based workflow. It selects a conflict-free batch of tasks, dispatches parallel workers in a shared worktree, verifies the result, opens a PR, and hands off to the review cycle.

```bash
orbit run ship
```

Source: [`orbit/orbit-core/assets/jobs/job_parallel_task_pipeline.yaml`](orbit/orbit-core/assets/jobs/job_parallel_task_pipeline.yaml)

### `job_local_task_pipeline`

A local-only workflow. It plans, implements, and commits directly without opening a PR or entering the GitHub review loop.

```bash
orbit run ship-local
```

Source: [`orbit/orbit-core/assets/jobs/job_local_task_pipeline.yaml`](orbit/orbit-core/assets/jobs/job_local_task_pipeline.yaml)


### `job_batch_review_cycle`

Reviews a batch PR against task acceptance criteria, syncs review threads to GitHub, and either merges on approval or enters the fix loop.

```bash
orbit job run job_batch_review_cycle --input '{"base":"main","pr_number":"42"}'
```

### `job_review_tasks`

Runs standalone task review for items in `proposed` or `review`.

```bash
orbit run review
```

---

## Architecture

Orbit is structured as a layered set of Rust crates. Lower layers have no knowledge of higher layers.

```text
orbit-types          (leaf: shared types, OrbitError, ID generation)
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

Orbit uses a multi-model strategy to balance reasoning depth and throughput:

| Model | Role | Rationale |
| :--- | :--- | :--- |
| **Claude (Opus)** | Planning, dispatch | Strong higher-order reasoning and planning |
| **Codex (gpt-5.4)** | Implementation, code generation, code review | Strong execution quality and code review performance |

You can override model choices in the job definitions under `.orbit/jobs/jobs`.

---

## Persistence And Repo Hygiene

Orbit state is local by default.

- Global state lives under `~/.orbit/`
- Workspace execution state lives under `<repo>/.orbit/`
- Scoreboards, tasks, diagnostics, and job runs are workspace-scoped artifacts

If you want Orbit state to persist across machines without polluting normal git history, use an overlay or similar mechanism. One example is tracked here:

- [monodev Orbit state overlay](https://github.com/danieljhkim/orbit/tree/monodev/persist/persist/stores/orbit-states/overlay/.orbit)

---

## Current Status

Orbit is still a work in progress.

- Core local execution primitives are usable today
- The intended workflows are increasingly stable
- Some product surfaces and derived artifacts are still evolving
- Production or multi-machine deployments are not yet recommended

Orbit is best viewed today as a serious local workflow engine for agent-assisted software delivery, not as a hosted orchestration platform or a replacement for GitHub/Jira/Linear.

---

## Contributing

Contributions focused on execution primitives, state management, workflow ergonomics, docs, and tool-calling interfaces are welcome. Open an issue or submit a pull request for review.
