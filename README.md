# Orbit — The audit log for your AI coding agents

<p align="center">
  <img src="docs/assets/orbit-dashboard-hero.gif" alt="Orbit dashboard: task backlog, agent execution, and live audit log" width="600" />
</p>

<p align="center">
  <em>The Orbit dashboard (<code>orbit web serve</code>) — task backlog, live audit log, per-agent scoreboard.</em>
</p>

**Orbit is a durable, intent-tracked, auditable task layer for developers driving AI coding agents at high volume — local-first by design, with a path to team-scale automation as trust in agents matures.**

You drive multiple coding agents (Claude Code, Codex CLI, Gemini CLI, and any OpenAI-compatible or Ollama-served model) against real code. Ideas accumulate faster than any session can hold, work spans branches and weeks, and six months from now you have to remember *why* an agent wrote a given line. Agent vendors solve in-session execution. Orbit is the layer above — local-first state that turns individual agent sessions into a coherent, auditable body of work.

Full positioning, commercial model, and roadmap: [docs/POSITIONING.md](docs/POSITIONING.md).

---

## Primary Features

- **Durable, intent-tracked task layer.** Lifecycle (`proposed → backlog → in-progress → review → done`) survives sessions, branches, and weeks. Every commit carries the `task_id`; `orbit task show` reconstructs prompt, plan, execution trace, and review threads months later.

- **Auditability.** Every tool call, provider request/response, and task transition is a structured, queryable event with agent identity attached. Append-only, tamper-evident, exportable. → [docs/design/auditability/](docs/design/auditability/)

- **Knowledge-graph–aware tooling.** Agents query a parsed, content-addressed graph (symbols, imports, callers, implementors) instead of grep. Branch-scoped, safe for parallel rebuild. The graph is what makes audit cheap to populate; benchmark in [`benchmarks/graph/`](benchmarks/graph/). → [docs/design/knowledge-graph/](docs/design/knowledge-graph/)

- **Conflict-aware parallel execution.** Each agent run is dispatched into its own git worktree, and the gate pipeline reserves task `context_files` as locks before fanning out — overlapping reservations are rejected up front rather than producing merge conflicts later. Locks auto-release when their owning run reaches a terminal state. Agents themselves do not call the lock APIs; coordination happens at the workflow plane. → [docs/design/activity-job/](docs/design/activity-job/)

> **Platform:** OS-level sandbox enforcement is **macOS only** (via `sandbox-exec`). On Linux/Windows, FS policies still apply as in-process guards for HTTP-tool calls; the spawned agent subprocess runs without OS-level isolation.

---

## Quick Start

**Prerequisites:** at least one supported agent CLI (Codex, Claude Code, or Gemini CLI), authenticated. For PR-based execution, `gh` installed and authenticated; otherwise use `--mode local`.

```bash
# install
curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/agent-main/install.sh | sh
# or: brew install danieljhkim/tap/orbit
# or, in Claude Code:
#   /plugin marketplace add danieljhkim/orbit
#   /plugin install orbit
# Claude Code plugin install takes ~30s on first use (downloads the
# platform-matched orbit binary from GitHub Releases via @orbit-tools/cli).
# After install you get the Orbit MCP tool surface (orbit.task.*,
# orbit.graph.*, etc.) plus the orbit skills and orchestration subagents.
# Verified weekly on macOS and Linux; Windows is not supported by the npm
# install path. Release procedure: docs/RELEASE.md.

# initialize
orbit init                                 # global state (~/.orbit)
cd <repo> && orbit workspace init --mcp    # workspace state + MCP integration

# launch interactive dashboard
orbit web serve

# create, approve, and ship a task
TASK_ID=$(orbit task add \
  --title "..." \
  --description "..." \
  --acceptance-criteria "..." \
  --workspace .)

# or simply ask an agent to create a task:
"Claude can you create an orbit task to refactor the authentication 
logic in ..."

orbit task approve "$TASK_ID"

orbit run ship-auto      # conflict-aware flush of the backlog tasks to PR
```

Full command reference: `orbit --help` and [orbit-cli.com](https://orbit-cli.com).

---

## Core Model

- **Task** — durable unit of work, versioned and auditable, scoped to a workspace.
- **Knowledge graph** — parsed structure of your codebase. Branch-scoped; safe for parallel rebuild.
- **Worktree** — each agent session runs in an isolated git worktree.
- **Locks** — explicit claims on files or code regions; reserved before dispatch to prevent overlapping edits.

Substrate primitives (`activity`, `job`, `policy`, `executor`, `tool`) are inspectable on purpose but not the product story.

---

## Workspace Layout (`.orbit/`)

`orbit workspace init` creates a `.orbit/` directory at the repo root. All workspace state lives here — the directory is the source of truth, and removing it returns the workspace to a pre-init state.

```
.orbit/
├── tasks/        # task bundles (projections of ~/.orbit/tasks/workspaces/<workspace-id>/)
├── knowledge/    # parsed knowledge graph for this workspace
│   ├── graph/
│   │   ├── objects/        # content-addressed graph objects
│   │   ├── refs/heads/     # per-branch graph refs (safe for parallel rebuild)
│   │   ├── index/by-id/    # id → object lookups
│   │   ├── blobs/          # large payloads
│   │   └── graph_index.sqlite
│   ├── manifest.json       # current build manifest
│   ├── hashes.json         # file → content-hash map for incremental rebuild
│   └── refresh_state.json  # last refresh metadata + refresh.lock
├── state/        # runtime state — append-only and rebuildable
│   ├── audit/         # append-only audit events (tool calls, transitions, provider I/O)
│   ├── job-runs/      # per-run metadata for each agent dispatch
│   ├── worktrees/     # worktree registry — tracks live agent sandboxes
│   ├── logs/          # agent + tool logs
│   ├── scoreboard/    # rolling counters (e.g. pr.json, task_review.json)
│   └── diagnostics/
├── resources/    # workflow definitions: activities, executors, jobs, policies
├── frictions/    # local friction log + tags.yaml
│
│   # lazily created on first use — not present immediately after init:
├── adrs/         # Architecture Decision Records (proposed/, accepted/, superseded/)
└── learnings/    # durable project learnings — pull-surface knowledge for agents
```

Three things to note:
- **`tasks/`** is a projection. Canonical task bundles live under `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/` so they survive repo moves; `.orbit/tasks/` is rebuildable from the canonical store. See [docs/design/task-artifacts/](docs/design/task-artifacts/).
- **`knowledge/graph/refs/heads/`** is per-branch on purpose, so concurrent rebuilds in separate worktrees do not race on a single pointer. See [docs/design/knowledge-graph/](docs/design/knowledge-graph/).
- **`adrs/`** and **`learnings/`** are committed *into* git — they are project memory, not local state. ADRs are appended via `orbit adr add` and transition through `proposed → accepted → superseded`. Learnings are appended via `orbit learning add` and pulled by agents through `orbit.learning.*` to surface project context on demand. Both directories are created the first time you add an entry.

Global state — credentials, the canonical task store, and cross-workspace config — lives under `~/.orbit/`, created by `orbit init`. The recommended `.gitignore` pattern is `.orbit/*` with `!.orbit/adrs/` and `!.orbit/learnings/` un-ignored, so local runtime state stays out of the repo while project memory stays in.

---

## Agent Tool Surface (MCP)

`orbit workspace init --mcp` registers the Orbit MCP server with the local agent CLI (Claude Code, Codex, Gemini). Names are canonically dot-separated (`orbit.task.add`); MCP clients that reject `.` see the underscored form (`orbit_task_add`) — both resolve to the same tool.

| Namespace | Tool | Purpose |
|---|---|---|
| **task** | `orbit.task.add` | Create a new task |
| | `orbit.task.update` | Mutate task fields (status, plan, acceptance criteria) |
| | `orbit.task.show` | Fetch full task detail |
| | `orbit.task.list` | List tasks filtered by status / scope |
| | `orbit.task.search` | Search tasks by text or metadata |
| | `orbit.task.delete` | Remove a task |
| | `orbit.task.start` | Transition into in-progress |
| | `orbit.task.approve` | Human approval gate (proposed → backlog) |
| | `orbit.task.reject` | Reject and close a task |
| | `orbit.task.lint` | Validate a draft against authoring rules |
| | `orbit.task.artifact.put` | Attach a generated artifact to a task |
| **review** | `orbit.task.review_thread.add` | Open a review thread on a task |
| | `orbit.task.review_thread.list` | List review threads on a task |
| | `orbit.task.review_thread.reply` | Reply to a thread |
| | `orbit.task.review_thread.resolve` | Close a thread |
| **graph** | `orbit.graph.search` | Find symbols / files in the parsed graph |
| | `orbit.graph.show` | Show a node by id |
| | `orbit.graph.overview` | Crate / module structural summary |
| | `orbit.graph.callers` | List callers of a symbol |
| | `orbit.graph.deps` | List outbound dependencies |
| | `orbit.graph.implementors` | List trait implementors |
| | `orbit.graph.refs` | List references to a symbol |
| | `orbit.graph.history` | Git history for a symbol |
| | `orbit.graph.pack` | Bundle a connected slice of the graph for a prompt |
| **semantic** | `orbit.semantic.search` | Embedding search over graph content |
| | `orbit.semantic.related` | Find semantically related nodes |
| **adr** | `orbit.adr.add` | Author an Architecture Decision Record |
| | `orbit.adr.update` | Edit an ADR |
| | `orbit.adr.show` | Fetch an ADR |
| | `orbit.adr.list` | List ADRs by status |
| | `orbit.adr.supersede` | Mark an ADR superseded by another |
| **learning** | `orbit.learning.add` | Author a project learning |
| | `orbit.learning.update` | Edit a learning |
| | `orbit.learning.show` | Fetch a learning |
| | `orbit.learning.list` | List learnings by tag / scope |
| | `orbit.learning.search` | Search learnings by path, tag, or text |
| | `orbit.learning.supersede` | Mark a learning superseded |
| | `orbit.learning.prune` | Report or archive stale learnings |
| | `orbit.learning.reindex` | Rebuild the SQLite envelope index from YAML |
| **friction** | `orbit.friction.add` | Record an operational friction |
| | `orbit.friction.update` | Edit a friction |
| | `orbit.friction.show` | Fetch a friction |
| | `orbit.friction.list` | List frictions by tag / status |
| | `orbit.friction.stats` | Aggregate frictions by tag and recency |
| | `orbit.friction.resolve` | Mark a friction resolved |
| | `orbit.friction.reject` | Reject a friction |
| | `orbit.friction.delete` | Delete a friction |

Substrate-internal namespaces (`orbit.state.*`, `orbit.pipeline.*`, `orbit.policy.*`, `orbit.task.locks.*`, `orbit.graph.{add,move,write,delete}`) are also registered but are called by the workflow plane, not by agent prompts. Full schemas are discoverable via the MCP `tools/list` call against the running server.

---

## Current Status

Orbit is v0.5.x — work in progress.

- Core local execution, graph build/query, and audit infrastructure are usable today.
- The execution substrate shows more internal machinery than the final product should; some historical CLI surfaces remain even though they're no longer central.
- Production or multi-machine deployments are not yet recommended.

Intentional technical debt on the path toward a tighter product focused on the audit and task layer.

---

## Commercial Model

OSS (this repo, MIT/Apache 2.0) is the full solo-wedge experience — free forever for self-hosted individuals and small teams. **Orbit Team** is a planned hosted multi-tenant SKU for engineering organizations. Full structure: [POSITIONING § Commercial model](docs/POSITIONING.md#commercial-model-open-core-two-tiers).

---

## Contributing

Contributions especially welcome on graph-aware scheduling, locking, worktree/session management, execution primitives, reconciliation, audit coverage, and tool-calling interfaces.

Before contributing: [docs/design/CONVENTIONS.md](docs/design/CONVENTIONS.md) and [CLAUDE.md](CLAUDE.md).
