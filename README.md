# Orbit — The engineering framework for your AI coding agents

<p align="center">
  <img src="docs/assets/orbit-dashboard-hero.gif" alt="Orbit dashboard: task backlog, agent execution, and live audit log" width="600" />
</p>

<p align="center">
  <em>The Orbit dashboard (<code>orbit web serve</code>) — task backlog, live audit log, per-agent scoreboard.</em>
</p>

**Orbit brings engineering rigor to AI-assisted coding. Tasks for every change, ADRs for load-bearing decisions, structured audit of every tool call and provider exchange, conflict-aware parallel dispatch — local-first.**

You drive Claude Code, Codex, Grok Build, or Gemini CLI against real code, often in parallel. Agents make it easy to skip the disciplines that keep code maintainable — no plan, no decision record, no audit trail, just prompt-and-merge. Six months later you can't reconstruct why an agent wrote a given line. Orbit makes those disciplines cheap and enforces them by default: tasks before edits, ADRs for load-bearing decisions, every tool call landing in a structured audit log, parallel runs sandboxed into worktrees with file-level locks.

The constraints are the point — they're what keep agent-assisted code shippable at volume. And the history of decisions lives right alongside the code, so that agents (and you) can reconstruct how the code came to be.

---

## Primary Features

- **Durable, intent-tracked task layer.** Lifecycle (`proposed → backlog → in-progress → review → done`) survives sessions and branches; every commit carries the `task_id`, so `orbit task show` reconstructs prompt, plan, execution trace, and review threads months later. → [docs/design/task-artifacts/](docs/design/task-artifacts/)

- **ADRs as first-class state.** Capture load-bearing decisions as ADR artifacts with status lifecycle (`proposed → accepted → superseded`), owner, related_tasks/features, and supersession chains — authored and queried via `orbit.adr.*`, cross-referenced from task IDs and commit messages. → [docs/design/adr-artifact/](docs/design/adr-artifact/)

- **Design docs with decay checks.** Scaffold, inspect, and lint `docs/design/<feature>/` folders through `orbit.design.*`; `orbit design check` flags docs whose `**Last updated:**` predates referenced `crates/...rs` code, with conventions anchored in [docs/design/CONVENTIONS.md](docs/design/CONVENTIONS.md).

- **Structured audit log.** Every tool call, provider request/response, and task transition becomes a queryable event with agent identity attached — append-only, tamper-evident, exportable. → [docs/design/auditability/](docs/design/auditability/)

- **Knowledge-graph–aware tooling.** Agents query a parsed, content-addressed graph (symbols, imports, callers, implementors) instead of grep. Branch-scoped and safe for parallel rebuild; numbers in [`benchmarks/graph/`](benchmarks/graph/). → [docs/design/knowledge-graph/](docs/design/knowledge-graph/)

- **Conflict-aware parallel execution.** For `orbit run ship`, each agent run lands in its own git worktree per task, and the gate pipeline reserves task `context_files` as locks before fanning out, rejecting overlapping reservations up front instead of producing merge conflicts later (see [merge throughput chart](docs/assets/merge-throughput.png)). → [docs/design/activity-job/](docs/design/activity-job/)

- **Sandboxed-by-default execution.** Dispatched agent CLIs run under an OS-level sandbox out of the box — FS access scoped to the worktree, network egress gated by per-activity policy. **macOS only today** (via `sandbox-exec`); on Linux/Windows the agent subprocess runs unsandboxed, with in-process FS guards still covering HTTP tools. → [docs/design/policy-sandbox](docs/design/policy-sandbox/)

---

## Quick Start

### Setup via Agent Prompt (clone & build) - Recommended

Cloning is the recommended and best way to get started with Orbit. Curl/brew/plugin paths give you a binary; cloning gives you a customizable framework to mold into your team's conventions. No need to contribute back to Orbit unless you want to, you can just fork it.

- If you need to build your custom workflow, ask the agent directly.
- If you don't like any orbit conventions, ask the agent to tweak it.
- If something doesn't work, ask the agent to fix it.
- If you need a new feature, ask the agent to add it.
- If you are unsure about any orbit features, ask the agent to help you.

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

### Manual Setup (old school way)

Not recommended unless you're a contrarian or you're in a highly restricted environment where you can't clone things. This way is harder and less flexible - really makes little sense to choose this route. But if you must:

**Prerequisites:** at least one supported agent CLI (Codex, Claude Code, or Gemini CLI), authenticated. For PR-based workflows (i.e., `orbit run ship` in the default `--mode pr`), `gh` installed and authenticated; otherwise use `--mode local`.

<details>
<summary><strong>Manual setup commands</strong> — copy these into your terminal (click to expand)</summary>

```bash
# install
curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | sh
# or: brew install danieljhkim/tap/orbit
# or, in Claude Code:
#   /plugin marketplace add danieljhkim/orbit
#   /plugin install orbit

# initialize
orbit init                                 # global state (~/.orbit)
cd <repo> && orbit workspace init --mcp    # workspace state + MCP integration

# create, approve, and ship a task
TASK_ID=$(orbit task add \
  --title "..." \
  --description "..." \
  --acceptance-criteria "..." \
  --workspace .)

# or simply ask an agent to create a task:
# "Claude can you create an orbit task to refactor the authentication logic in ..."

orbit task approve "$TASK_ID"

# conflict-aware, parallel flush of the backlog tasks to PRs
orbit run ship

# launch interactive dashboard
orbit web serve
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

## Claude Code Plugin vs CLI

Two install surfaces. The CLI gives you the full power of Orbit. Choose the plugin if you just want a taste. Plugin will strap orbit's MCP tools automatically.

```bash
/plugin marketplace add danieljhkim/orbit

# Install the plugin
/plugin install orbit
```

<details>
<summary><strong>Plugin vs. CLI</strong> — (click to expand)</summary>

|   | **Claude Code plugin** | **CLI (curl / brew)** |
|---|---|---|
| Install | `/plugin install orbit` (after `/plugin marketplace add danieljhkim/orbit`) | `curl … \| sh` or `brew install danieljhkim/tap/orbit` |
| Orbit binary | Lives inside the plugin sandbox (not on `$PATH`) | Installed on `$PATH` |
| MCP registration | Automatic | Manual: `orbit workspace init --mcp` per workspace |
| Web dashboard (`orbit web serve`) | No | Yes |
| Works with Codex / Gemini CLI | No (Claude Code only) | Yes |
| workflows (i.e. `orbit run ship`) | No | Yes |

</details>

---

## Agent Skills

`orbit workspace init` seeds skill files under `~/.orbit/skills/` and symlinks them into `~/.claude/skills/` and `~/.agents/skills/`, so Claude Code, Codex, and Gemini CLI discover them at session start with no per-agent configuration. The router skill (`orbit`) classifies intent; workflow-specific skills do the work:

- `orbit-create-task` — author a task with strong acceptance criteria
- `orbit-execute-task` — carry an approved task through implementation and review
- `orbit-review-task` — file findings on another agent's work without transitioning status
- `orbit-adr` — author, accept, or supersede an Architecture Decision Record
- `orbit-graph` — query the parsed knowledge graph (callers, implementors, refs)
- `orbit-semantic` — find tasks by topic; dedup and related-task lookups
- `orbit-debug-job-failure` — diagnose failed, stuck, or cancelled runs
- `orbit-track-issues` — capture agent-self-reported friction with Orbit tooling itself

`orbit skill doctor` flags drift between the local copy and the upstream definition. Edit any seeded `SKILL.md` to customize behavior for your team.

---

## Agent Tool Surface (MCP)

`orbit workspace init --mcp` registers the Orbit MCP server with the local agent CLI (Claude Code, Codex, Gemini), same as plugin. Expand below to see the full tool surface.

<details>
<summary><strong>Full tool reference</strong> — task, review, graph, semantic, adr, design, learning, friction (click to expand)
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
| **design** | `orbit.design.init` | Scaffold a feature design-doc folder |
| | `orbit.design.list` | List design-doc feature folders |
| | `orbit.design.show` | Fetch one design-doc feature summary |
| | `orbit.design.check` | Return structured stale-doc findings |
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

---

## Workspace Layout of `.orbit`

- `orbit workspace init` creates a `.orbit/` directory at the repo root. Workspace-local state lives there; removing the directory returns the workspace to a pre-init state.
- `orbit init` creates a `.orbit/` directory in the user's home (`~/.orbit/`). User-scoped state lives there; removing the directory returns the user environment to a pre-init state.

```
.orbit/                          # workspace-local (safe to delete → clean slate)
├── config.yaml                  # workspace_id + config
├── tasks/                       # symlinks → ~/.orbit/tasks/workspaces/<id>/
├── adrs/                        # proposed/, accepted/, superseded/
├── learnings/                   # your team's durable knowledge
├── frictions/                   # local friction log + tags.yaml
├── knowledge/                   # parsed graph artifacts
├── resources/                   # activities, jobs, executors, policies (customizable)
└── state/
    ├── audit/                   # append-only JSONL events
    ├── job-runs/                # per-run metadata + step traces
    ├── worktrees/               # live git worktrees for agent runs
    ├── logs/                    # captured agent stdout/stderr
    └── scoreboard/              # rolling counters (PRs, reviews, etc.)

~/.orbit/                        # global (machine-level, survives repo moves)
├── tasks/
│   ├── index.sqlite             # authority for ORB-XXXXX IDs
│   └── workspaces/<workspace-id>/<task-id>/   # canonical task bundles
├── skills/                      # SKILL.md files (routable via MCP)
├── embed/                       # semantic companion binary + models
└── config.toml                  # global settings
```

Couple things to note:
- **`tasks/`** is a projection. Canonical task bundles live under `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/` so they survive repo moves; `.orbit/tasks/` is rebuildable from the canonical store. See [docs/design/task-artifacts/](docs/design/task-artifacts/).

- Global state — credentials, the canonical task store, and cross-workspace config — lives under `~/.orbit/`, created by `orbit init`. The recommended `.gitignore` pattern is `.orbit/*` with `!.orbit/adrs/` and `!.orbit/learnings/` un-ignored, so local runtime state stays out of the repo while project memory stays in.

---

## Current Status

Orbit is v0.5.x — work in progress.

- Core local execution, graph build/query, workflows, MCP, tasks, reviews, ADRs, frictions, and audit infrastructure are usable today.

---

## Contributing

Contributions especially welcome on graph-aware scheduling, locking, worktree/session management, execution primitives, reconciliation, audit coverage, and tool-calling interfaces.

Before contributing: [docs/design/CONVENTIONS.md](docs/design/CONVENTIONS.md) and [CLAUDE.md](CLAUDE.md).

---

## License

MIT
