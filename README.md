# Orbit — The engineering framework for your AI coding agents

<p align="center">
  <img src="docs/assets/orbit-dashboard-hero.gif" alt="Orbit dashboard: task backlog, agent execution, and live audit log" width="600" />
</p>

<p align="center">
  <em>The Orbit dashboard (<code>orbit web serve</code>) — task backlog, live audit log, per-agent scoreboard.</em>
</p>

**Orbit brings engineering rigor to AI-assisted coding. Tasks for every change, ADRs for load-bearing decisions, structured audit of every tool call and provider exchange, conflict-aware parallel dispatch — local-first.**

You drive Claude Code, Codex, or Gemini CLI against real code, often in parallel. Agents make it easy to skip the disciplines that keep code maintainable — no plan, no decision record, no audit trail, just prompt-and-merge. Six months later you can't reconstruct why an agent wrote a given line, and parallel runs collide because nothing reserved the files. Orbit makes those disciplines cheap and enforces them by default: tasks before edits, ADRs for load-bearing decisions, every tool call landing in a structured audit log, parallel runs sandboxed into worktrees with file-level locks. The constraints are the point — they're what keep agent-assisted code shippable at volume.

---

## Primary Features

- **Durable, intent-tracked task layer.** Lifecycle (`proposed → backlog → in-progress → review → done`) survives sessions and branches; every commit carries the `task_id`, so `orbit task show` reconstructs prompt, plan, execution trace, and review threads months later. → [docs/design/task-artifacts/](docs/design/task-artifacts/)

- **ADRs as first-class state.** Capture load-bearing decisions as ADR artifacts with status lifecycle (`proposed → accepted → superseded`), owner, related_tasks/features, and supersession chains — authored and queried via `orbit.adr.*`, cross-referenced from task IDs and commit messages. → [docs/design/adr-artifact/](docs/design/adr-artifact/)

- **Auditability.** Every tool call, provider request/response, and task transition becomes a structured, queryable event with agent identity attached — append-only, tamper-evident, exportable. → [docs/design/auditability/](docs/design/auditability/)

- **Knowledge-graph–aware tooling.** Agents query a parsed, content-addressed graph (symbols, imports, callers, implementors) instead of grep. Branch-scoped and safe for parallel rebuild; numbers in [`benchmarks/graph/`](benchmarks/graph/). → [docs/design/knowledge-graph/](docs/design/knowledge-graph/)

- **Conflict-aware parallel execution workflow** For `orbit run ship-auto`, each agent run lands in its own git worktree per task, and the gate pipeline reserves task `context_files` as locks before fanning out, rejecting overlapping reservations up front instead of producing merge conflicts later. [see merge-throughput](docs/assets/merge-throughput.png). → [docs/design/activity-job/](docs/design/activity-job/)

- **Sandboxed-by-default execution.** Dispatched agent CLIs run under an OS-level sandbox out of the box — FS access scoped to the worktree, network egress gated by per-activity policy. **macOS only today** (via `sandbox-exec`); on Linux/Windows the agent subprocess runs unsandboxed, with in-process FS guards still covering HTTP tools. → [docs/design/policy-sandbox](docs/design/policy-sandbox/)

---

## Quick Start

### Setup via Agent Prompt (clone & build) - Recommended

Cloning is the recommended and best way to get started with Orbit. Curl/brew/plugin paths give you a binary; cloning gives you a customizable framework to mold into your team's conventions. No need to contribute back to Orbit unless you want to, you can just fork it.

- If you need to build your custom workflow, ask the agent directly.
- If you don't like any orbit conventions, ask the agent to tweak it.
- If something doesn't work, ask the agent to fix it.
- If you need a new feature, ask the agent to add it.

Paste the prompt below into your agent (Claude Code, Codex CLI, or Gemini CLI) **from inside the repo where you want to use Orbit**. The agent clones Orbit, builds from source, sets up MCP, and reads the key docs so it can drive the workflow on your behalf afterwards.

<details>
<summary><strong>Agent setup prompt</strong> — copy this into your agent (click to expand)</summary>

> Clone https://github.com/danieljhkim/orbit, build and install the `orbit` CLI from source, then set up Orbit on this current repo. Become an expert in Orbit's model along the way.
>
> 1. Ask me where to clone the Orbit repo (suggest something tweakable like `~/code/orbit`). Clone it there.
> 2. From the cloned repo, run `make install`. This builds with cargo and copies the `orbit` binary to `$INSTALL_BIN_DIR` (default: `~/.cargo/bin`). Confirm the install path with me before running. Verify with `orbit --version`.
> 3. Run `orbit init` to initialize global state at `~/.orbit`.
> 4. From this current working directory (NOT the Orbit clone), run `orbit workspace init --mcp`. This creates `.orbit/` here and auto-registers Orbit's MCP server with installed agent CLIs (Claude Code, Codex, Gemini).
> 5. Read these files in the cloned Orbit repo to internalize the model and conventions:
>    - `README.md` — feature surface, install model, plugin vs CLI
>    - `docs/POSITIONING.md` — what Orbit is for, what it isn't
>    - `CLAUDE.md` — agent operating rules (commit timing, task ID convention, lint constraints)
>    - `ARCHITECTURE.md` — crate layering and dependency rules
>    - `docs/design/CONVENTIONS.md` — design-doc structure
> 6. Report what you ran, what you read, and the output of `orbit task list` + `orbit semantic stats`. If any step failed, **stop and ask me** before continuing.
>
> Don't run anything destructive (overwriting files, modifying shell config) without confirming. If `make install` would write outside `~/.cargo/bin`, ask me first.

</details>
<br>

### Manual Setup (not recommended) 

**Prerequisites:** at least one supported agent CLI (Codex, Claude Code, or Gemini CLI), authenticated. For PR-based workflows (i.e., `orbit run ship-auto`), `gh` installed and authenticated; otherwise use `--mode local`.

<details>
<summary><strong>Manual setup commands</strong> — copy these into your terminal (click to expand)</summary>

```bash
# install
curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/agent-main/install.sh | sh
# or: brew install danieljhkim/tap/orbit
# or, in Claude Code:
#   /plugin marketplace add danieljhkim/orbit
#   /plugin install orbit

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

orbit run ship-auto # conflict-aware, parallel flush of the backlog tasks to PRs
```

</details>
<br>

Full command reference: `orbit --help` and [orbit-cli.com](https://orbit-cli.com).


---

## Semantic Search (optional)

Opt-in hybrid (embedding + BM25) search over task fields via `orbit.semantic.search` / `orbit.semantic.related`. **Scope today is tasks only** — graph, ADRs, learnings, and code are not indexed. The embedder runs as a separate companion subprocess, so semantic search has zero cost when unused.

```bash
orbit semantic install    # one-time: download companion + default model (bge-small)
orbit semantic reindex    # backfill existing tasks
orbit semantic search "race in the scheduler when locks overlap"
```

After install, task writes are embedded automatically in the background; `reindex` is only needed for the initial backfill. Companion + models live under `~/.orbit/embed/`; the per-workspace index at `.orbit/state/semantic.db`.

---

## Plugin vs CLI

Two install surfaces. Same binary and MCP tool surface underneath — the difference is what Claude Code wires up for you.

|   | **Claude Code plugin** | **CLI (curl / brew)** |
|---|---|---|
| Install | `/plugin install orbit` (after `/plugin marketplace add danieljhkim/orbit`) | `curl … \| sh` or `brew install danieljhkim/tap/orbit` |
| Orbit binary | Lives inside the plugin sandbox (not on `$PATH`) | Installed on `$PATH` |
| MCP registration | Automatic | Manual: `orbit workspace init --mcp` per workspace |
| Web dashboard (`orbit web serve`) | No | Yes |
| Works with Codex / Gemini CLI | No (Claude Code only) | Yes |
| workflows (i.e. `orbit run ship-auto`) | No | Yes |

**On Claude Code, install the plugin** — it wires MCP automatically and ships subagents that don't exist on the CLI path. Add the CLI alongside if you also want the dashboard (`orbit web serve`) or terminal-driven workflows; the two share `~/.orbit/` global state.

**On Codex or Gemini CLI, use the CLI install.** `orbit workspace init --mcp` auto-detects installed MCP clients and registers Orbit with each; the MCP tool surface is identical, and `orbit init` seeds the skill set into the workspace.

---

## Agent Tool Surface (MCP)

`orbit workspace init --mcp` registers the Orbit MCP server with the local agent CLI (Claude Code, Codex, Gemini). Names are canonically dot-separated (`orbit.task.add`); MCP clients that reject `.` see the underscored form (`orbit_task_add`) — both resolve to the same tool.

<details>
<summary><strong>Full tool reference</strong> — task, review, graph, semantic, adr, learning, friction (click to expand)
</summary>

| Namespace | Tool | Purpose |
|---|---|---|
| **task** | `orbit.task.add` | Create a new task |
| | `orbit.task.update` | Mutate task fields (status, plan, acceptance criteria) |
| | `orbit.task.show` | Fetch full task detail |
| | `orbit.task.list` | List tasks filtered by status / scope |
| | `orbit.task.search` | Search tasks by text or metadata |
| | `orbit.task.start` | Transition into in-progress |
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
| **semantic** | `orbit.semantic.search` | Hybrid (embedding + BM25) search over task fields — title, description, plan, acceptance, execution summary |
| | `orbit.semantic.related` | Find tasks semantically similar to a given task |
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
| | `orbit.friction.delete` | Delete a friction |

</details>
<br>

Substrate-internal namespaces (`orbit.state.*`, `orbit.pipeline.*`, `orbit.policy.*`, `orbit.task.locks.*`, `orbit.graph.{add,move,write,delete}`) are also registered but are called by the workflow plane, not by agent prompts. Full schemas are discoverable via the MCP `tools/list` call against the running server.

---

## Workspace Layout (`.orbit/`)

`orbit workspace init` creates a `.orbit/` directory at the repo root. All workspace state lives here — the directory is the source of truth, and removing it returns the workspace to a pre-init state.

```
.orbit/
├── tasks/        # task bundles (projections of ~/.orbit/tasks/workspaces/<workspace-id>/)
├── knowledge/    # parsed knowledge graph for this workspace
├── state/        # runtime state — append-only and rebuildable
│   ├── audit/         # append-only audit events (tool calls, transitions, provider I/O)
│   ├── job-runs/      # per-run metadata for each agent dispatch
│   ├── worktrees/     # worktree registry — tracks live agent sandboxes
│   ├── logs/          # agent + tool logs
│   ├── scoreboard/    # rolling counters (e.g. pr.json, task_review.json)
│   └── diagnostics/
├── resources/    # workflow definitions: activities, executors, jobs, policies
├── frictions/    # local friction log + tags.yaml
├── adrs/         # Architecture Decision Records (proposed/, accepted/, superseded/)
└── learnings/    # durable project learnings — pull-surface knowledge for agents
```

Couple things to note:
- **`tasks/`** is a projection. Canonical task bundles live under `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/` so they survive repo moves; `.orbit/tasks/` is rebuildable from the canonical store. See [docs/design/task-artifacts/](docs/design/task-artifacts/).

- Global state — credentials, the canonical task store, and cross-workspace config — lives under `~/.orbit/`, created by `orbit init`. The recommended `.gitignore` pattern is `.orbit/*` with `!.orbit/adrs/` and `!.orbit/learnings/` un-ignored, so local runtime state stays out of the repo while project memory stays in.

---

## Current Status

Orbit is v0.5.x — work in progress.

- Core local execution, graph build/query, and audit infrastructure are usable today.
- The execution substrate shows more internal machinery than the final product should; some historical CLI surfaces remain even though they're no longer central.
- Production or multi-machine deployments are not yet recommended.

Intentional technical debt on the path toward a tighter product focused on the audit and task layer.

---

## Contributing

Contributions especially welcome on graph-aware scheduling, locking, worktree/session management, execution primitives, reconciliation, audit coverage, and tool-calling interfaces.

Before contributing: [docs/design/CONVENTIONS.md](docs/design/CONVENTIONS.md) and [CLAUDE.md](CLAUDE.md).

---

## License

MIT