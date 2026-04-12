# Orbit: Local-First Agentic Workflow Engine

Orbit is a local-first workflow engine for agent-driven software delivery. It helps humans and coding agents coordinate around structured tasks, reusable activities, and repeatable job pipelines directly inside a repository.

Orbit runs on top of agent CLIs such as Codex and Claude Code. No provider API keys are required by Orbit itself.

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

## Architecture

Orbit is structured as a layered set of Rust crates. Lower layers have no knowledge of higher layers.

```mermaid
flowchart LR
  CLI["orbit-cli"] --> Core["orbit-core"]
  Core --> Engine["orbit-engine"]
  Core --> Store["orbit-store"]
  Core --> Tools["orbit-tools"]
  Core --> Policy["orbit-policy"]
  Engine --> Agent["orbit-agent"]
  Engine --> Store
  Tools --> Exec["orbit-exec"]
  Tools --> Knowledge["orbit-knowledge"]
  Store --> Types["orbit-types"]
  Agent --> Types
  Core --> Types

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
