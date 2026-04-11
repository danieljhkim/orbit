# Orbit: Local-First Agentic Workflow Engine

Orbit is a local-first workflow engine for agent-driven software delivery. It helps humans and coding agents coordinate around structured tasks, reusable activities, and repeatable job pipelines directly inside a repository.

Orbit runs on top of agent CLIs such as Codex and Claude Code. No provider API keys are required by Orbit itself.

## Philosophy

> Minimize irrelevant decisions. Maximize determinism. Make every token count.

Orbit should make agents spend judgment on what matters: intent, correctness, tradeoffs, design, and risk. Agent opinions and decisions should directly support these values; if a choice does not reduce token usage, remove irrelevant discretion, or make the system more deterministic, Orbit should make that choice instead.

---

## Quick Start

**Prerequisites**: Rust, Codex CLI, Gemini CLI, and Claude Code

Suggestion: Get cheapest tier subscriptions for above 3, and milk every dollar out of it.

For the PR-based workflow (`orbit ship run`), you also need the GitHub CLI (`gh`) installed and authenticated. If you do not want to use GitHub or open pull requests, use `orbit ship run --local` instead.

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
orbit ship run

# or run a local-only workflow with no PR/review loop
orbit ship run --local
```

If you already know which tasks you want to run, pin them explicitly:

```bash
orbit ship run --tasks T123,T456 --parallelism 2 --base main
```

---

## Mental Model

Orbit has four core concepts:

- **Task**: a unit of work to be proposed, approved, implemented, reviewed, and tracked
- **Activity**: a reusable operation with a defined input/output shape
- **Job**: an ordered workflow made of activities and nested jobs
- **Workflow**: a human-friendly entrypoint over a common job pipeline, exposed through `orbit ship` and `orbit duel`

In practice:

- Humans review and approve work
- Agents create and implement tasks
- Orbit jobs orchestrate planning, execution, review, and merge

---

## Canonical Path

For most repositories, the intended path is:

1. Create tasks
2. Review and approve them
3. Run `orbit ship run`
4. Let Orbit dispatch agents, verify results, open a PR, review it, and merge when approved

Most users should start with the first-class workflows below rather than the lower-level job and activity internals.

Note: `orbit ship run` depends on GitHub-backed PR operations and therefore requires `gh` to be installed and logged in. `orbit ship run --local` does not.

---

## Who Uses Which Surface?

Orbit has different surfaces for different actors:

- **Humans** usually use the CLI directly: `orbit task ...`, `orbit ship ...`, `orbit duel ...`, `orbit metrics ...`
- **Agents** should use the Orbit tool surface: `orbit tool run ...`
- **Automations/jobs** operate through activities, runtime hooks, and registered tools

These surfaces are intentionally distinct because Orbit records provenance and execution context differently for humans, agents, and automation. If you are writing agent workflows, prefer the tool surface rather than shelling out to human-oriented CLI commands.

---

## First-Class Workflows

Orbit exposes a small, workflow-oriented top-level surface:

| Workflow | Command | Description |
| :--- | :--- | :--- |
| **ship** | `orbit ship run` | Select tasks, dispatch agents, verify results, open a PR, review, and merge; requires `gh` auth |
| **ship-local** | `orbit ship run --local` | Select tasks, dispatch agents, and commit locally without a PR |
| **duel** | `orbit duel run [task_id]` | Single-task cross-agent evaluation: a random permutation of implementer/reviewer/arbiter across agent families, scored into `.orbit/scoreboard/duel.json` |

Each duel run appends an entry to `.orbit/scoreboard/duel.json`. Inspect aggregates with `orbit duel score` (add `--by scope` or `--by ambiguity` to segment, `--role implementer` to filter, `--json` for raw output). The numbers feed back into agent selection for `ship`.

Optional flags:

- `orbit ship run --tasks T1,T2` pins a specific ship batch instead of auto-selecting from backlog
- `orbit ship run --parallelism N` controls the number of parallel workers
- `orbit ship run --base BRANCH` and `orbit duel run <task_id> --base BRANCH` override the base branch

Examples:

```bash
# auto-select tasks from backlog
orbit ship run

# pin specific tasks
orbit ship run --tasks T20260402-0352,T20260402-0406 --parallelism 2

# local-only pipeline
orbit ship run --local --base main

# inspect the latest ship run
orbit ship show

# run a duel against a specific task
orbit duel run T20260402-0352

# inspect the latest duel run
orbit duel show

# inspect duel scoreboard aggregates
orbit duel score --by scope
```

---

## CLI Surface

Orbit exposes a small set of top-level command groups:

```text
orbit [OPTIONS] <COMMAND>

Setup:
  init       Initialize the global Orbit root (~/.orbit)
  workspace  Initialize and manage workspaces
  config     Show or update Orbit configuration

Run workflows:
  ship       Ship tasks through the pipeline
  duel       Cross-agent scoring

Manage work:
  task       Create, update, and manage tasks
  tool       Manage and run Orbit tools
  skill      Manage agent skill definitions

Inspect:
  audit      Query the audit event log
  metrics    Inspect token, tool-call, and knowledge-pack metrics
```

`orbit job ...` and `orbit activity ...` still work for compatibility, but they are intentionally omitted from `orbit --help` and are not the documented primary surface.

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
- They are the engine behind the higher-level `orbit ship run` and `orbit duel run` workflows

---

## Default Jobs

Orbit currently ships with a small set of default jobs that cover planning, implementation, review, and merge.

### `job_parallel_task_pipeline`

The main PR-based workflow. It selects a conflict-free batch of tasks, dispatches parallel workers in a shared worktree, verifies the result, opens a PR, and hands off to the review cycle.

```bash
orbit ship run
```

Source: [`crates/orbit-core/assets/jobs/job_parallel_task_pipeline.yaml`](crates/orbit-core/assets/jobs/job_parallel_task_pipeline.yaml)

### `job_local_task_pipeline`

A local-only workflow. It plans, implements, and commits directly without opening a PR or entering the GitHub review loop.

```bash
orbit ship run --local
```

Source: [`crates/orbit-core/assets/jobs/job_local_task_pipeline.yaml`](crates/orbit-core/assets/jobs/job_local_task_pipeline.yaml)


### `job_batch_review_cycle`

Reviews a batch PR against task acceptance criteria, syncs review threads to GitHub, and either merges on approval or enters the fix loop.

This job is triggered internally by the ship pipeline after a PR is opened.

### `job_review_tasks`

Runs standalone task review for items in `proposed` or `review`.

This job is used internally when Orbit performs standalone task review.

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
