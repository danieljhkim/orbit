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

- **ADRs as first-class state.** Capture load-bearing decisions as ADR artifacts with status lifecycle (`proposed → accepted → superseded`), owner, related_tasks/features, and supersession chains — authored and queried via `orbit.adr.*`, cross-referenced from task IDs and commit messages. → [.orbit/adrs/](.orbit/adrs/)

- **Shared learnings, smarter agents.** Non-obvious knowledge — gotchas, root causes, validated approaches — captured once as scoped `L<date>-N` records that inject into any agent's context automatically when relevant code is touched (engine pre-prompt, MCP sidecar, optional `PreToolUse` hook). Authored via `orbit.learning.*`, checked into git so what one agent learns the next one inherits. → [docs/design/project-learnings/](docs/design/project-learnings/)

- **Structured audit log.** Every tool call, provider request/response, and task transition becomes a queryable event with agent identity attached — append-only, tamper-evident, exportable. → [docs/design/auditability/](docs/design/auditability/)

- **Code-graph–aware tooling.** Agents query a parsed SQLite code index (symbols, imports, references, implementors) instead of grep. Per-worktree and regenerable on demand; numbers in [`benchmarks/graph/`](benchmarks/graph/). → [docs/design/orbit-graph/](docs/design/orbit-graph/)

- **Conflict-aware parallel execution.** For `orbit run ship`, each agent run lands in its own git worktree per task, and the gate pipeline reserves task `context_files` as locks before fanning out, rejecting overlapping reservations up front instead of producing merge conflicts later (see [merge throughput chart](docs/assets/merge-throughput.png)). → [docs/design/activity-job/](docs/design/activity-job/)

- **Sandboxed-by-default execution.** Dispatched agent CLIs run under an OS-level sandbox out of the box — FS access scoped to the worktree, network egress gated by per-activity policy. **macOS only today** (via `sandbox-exec`); on Linux/Windows the agent subprocess runs unsandboxed, with in-process FS guards still covering HTTP tools. → [docs/design/policy-sandbox](docs/design/policy-sandbox/)

- **Pluggable executors, no fork.** Register a homegrown out-of-process executor — any binary or script — through a config-only `executor_type: external` def: Orbit spawns it, streams the JSON request envelope over stdin, and maps its exit code to the activity outcome. No recompile, no linking, language-agnostic; copy the [example def](crates/orbit-core/assets/executors/external.example.yaml) to get started. → [docs/design/executors/specs/external-executor-protocol.md](docs/design/executors/specs/external-executor-protocol.md)

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

> You are helping me set up Orbit, a local governance and audit layer for coding agents.
>
> I am a staff/principal/founding engineer who already uses multiple coding agents heavily (Claude Code, Codex, Gemini, Aider, etc.) and has started to feel the long-term maintainability cost of moving fast without enough structure.
>
> Your job is to install and configure Orbit inside this repository so that I can keep using my existing agents while gaining durable tasks, structured audit, ADRs, safe parallel execution, and a code knowledge graph.
>
> Follow these steps carefully:
>
> 1. Ask me where I want to clone the Orbit repository (suggest something like `~/code/orbit` or `~/dev/orbit`).
> 2. Verify the Rust toolchain. Run `cargo --version` and `rustc --version`. Orbit declares `rust-version = "1.89"` (MSRV), so I need Rust **1.89 or newer**. If cargo is missing, or rustc is older than 1.89, **stop and ask me before installing anything** — the canonical path is `rustup` (`curl https://sh.rustup.rs | sh`), but that modifies shell profile, so I want to confirm first. If rustup is already installed but the toolchain is old, suggest `rustup update stable` and confirm before running.
> 3. Clone `https://github.com/danieljhkim/orbit` into the location from step 1, then run `make install`. This builds with cargo and copies the `orbit` binary to `$INSTALL_BIN_DIR` (default: `~/.cargo/bin`). Confirm the install path with me before running. Verify with `orbit --version`.
> 4. Run `orbit init` to initialize global state at `~/.orbit`.
> 5. From *this* repository (not the Orbit clone), run `orbit workspace init --mcp`. This creates `.orbit/` here and auto-registers Orbit's MCP server with installed agent CLIs (Claude Code, Codex, Gemini).
> 6. Ask me whether to enable semantic search (**optional**). `orbit semantic install` downloads a small embedder companion plus the default bge-small model (lives under `~/.orbit/embed/`) and powers `orbit search <query> --hybrid` / `orbit search similar <task-id>` over tasks. It requires macOS arm64 or Linux x86_64/aarch64 with glibc >= 2.38; Intel macOS is unsupported for semantic search. Don't install without my OK. If I accept and tasks already exist in this workspace, also run `orbit semantic index` to backfill the corpus.
> 7. Read the key documents so you actually understand the model:
>    - `README.md` — feature surface, install model, plugin vs CLI
>    - `docs/POSITIONING.md` — what Orbit is for, what it isn't (especially "who this is for")
>    - `CLAUDE.md` — agent operating rules (commit timing, task ID convention, lint constraints)
>    - `ARCHITECTURE.md` — crate layering and dependency rules
>    - `docs/design/CONVENTIONS.md` — design-doc structure
>    - `docs/CONFIG.md` — config reference: crew/workflow/duel knobs and per-task crew override
> 8. After setup, run `orbit task list` and `orbit semantic stats` and show me the output.
> 9. Ask me what my first real task should be and create it properly using Orbit's task surface (use the `orbit-task` skill — it should be auto-discovered after step 5).
>
> Rules:
> - Never run destructive commands without explicit confirmation. Specifically: cloning, installing rustup, running `make install` outside `~/.cargo/bin`, and any shell-profile modification all need a confirmation prompt.
> - If anything is unclear or fails, stop and ask me.
> - Do not try to "make it simpler" or hide Orbit's conventions. I am choosing this because I want the discipline.
>
> Report back what you did and the current state of the workspace.

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
# or, in Codex CLI:
#   codex plugin marketplace add danieljhkim/orbit --ref main
#   codex plugin add orbit@orbit

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

orbit task update "$TASK_ID" --status backlog   # approve into the backlog

# conflict-aware, parallel flush of the backlog tasks to PRs
orbit run ship

# launch interactive dashboard — one view over every registered workspace,
# from any directory. The header selector preselects the workspace for the
# directory orbit was launched from (if it's registered) and otherwise opens
# on "All workspaces"; it shows the selected workspace's filesystem path
# (home-abbreviated to ~) beneath the dropdown, and the aggregate task view
# lists each task's workspace location in its Details box — so same-named
# workspaces are easy to tell apart.
orbit web serve

# ...or view a workspace running on another machine over an SSH tunnel
# (the dashboard stays loopback-only; auth is delegated to SSH)
orbit web connect my-server
```

</details>
<br>

Full command reference: `orbit --help` and [orbit-cli.com](https://orbit-cli.com).

Customizing crews (which model runs planner/implementer/reviewer), the base branch, and `duel-plan` candidates: see [docs/CONFIG.md](docs/CONFIG.md).

---

## Semantic Search (optional)

`orbit search` is the unified query surface for tasks, docs, learnings, and ADRs. It defaults to lexical matching. Opt into hybrid embedding ranking over task fields or indexed docs with `--hybrid`, or find cosine-neighbor tasks with `orbit search similar <task-id>`. The embedder runs as a separate companion subprocess, so semantic search has zero cost when unused.

The semantic companion is released for macOS arm64 and Linux x86_64/aarch64
with glibc >= 2.38 (Ubuntu 24.04 or equivalent). Intel macOS can run the
Orbit CLI, but semantic search is unsupported because there is no Intel macOS
companion asset.

```bash
orbit semantic install    # one-time: download companion + default model (bge-small)
orbit semantic index      # backfill existing tasks
orbit docs index          # backfill docs for --kind doc --hybrid
orbit search "race in the scheduler when locks overlap"
orbit search "race in the scheduler when locks overlap" --hybrid --kind task
orbit search "why tasks serialize ORB ids" --hybrid --kind doc
orbit search similar ORB-00042
```

After install, task writes are embedded automatically in the background; `orbit semantic index` is only needed for the initial task backfill. Docs are indexed explicitly with `orbit docs index`, which skips unchanged content hashes and sweeps stale doc paths. Companion + models live under `~/.orbit/embed/`; the per-workspace index is `.orbit/state/semantic.db`. Learnings and ADRs remain lexical even when `--hybrid` is set.

---

## Agent Plugins vs CLI

Orbit ships lightweight Claude Code and Codex plugins. The CLI gives you the full power of Orbit; choose a plugin when you want Orbit's MCP tools and shared skills strapped onto one agent without installing `orbit` on your `$PATH`.

```bash
# Claude Code
/plugin marketplace add danieljhkim/orbit
/plugin install orbit

# Codex CLI
codex plugin marketplace add danieljhkim/orbit --ref main
codex plugin add orbit@orbit

# Later, after an Orbit release:
codex plugin marketplace upgrade orbit
codex plugin add orbit@orbit
```

<details>
<summary><strong>Plugin vs. CLI</strong> — (click to expand)</summary>

|   | **Claude Code plugin** | **Codex plugin** | **CLI (curl / brew)** |
|---|---|---|---|
| Install | `/plugin install orbit` after `/plugin marketplace add danieljhkim/orbit` | `codex plugin add orbit@orbit` after `codex plugin marketplace add danieljhkim/orbit --ref main` | `curl … \| sh` or `brew install danieljhkim/tap/orbit` |
| Orbit binary | Lives inside the plugin sandbox (not on `$PATH`) | Lives inside the plugin cache (not on `$PATH`) | Installed on `$PATH` |
| MCP registration | Automatic in Claude Code | Automatic in Codex | Manual: `orbit workspace init --mcp` per workspace |
| Shared Orbit skills | Bundled from `plugin/skills/` | Bundled from `plugin/skills/` | Seeded by `orbit workspace init` |
| Web dashboard (`orbit web serve`) | No | No | Yes |
| Other agent CLIs | No, scoped to Claude Code | No, scoped to Codex | Yes |
| Workflows (i.e. `orbit run ship`) | No | No | Yes |

</details>

> **Cowork users:** Cowork launches the plugin's MCP server with its working directory and `CLAUDE_PROJECT_DIR` pointed at an internal scratchpad rather than your selected repo, so `orbit mcp serve` finds no workspace and exposes only the graph tools — `orbit_task_add` and the rest of the task/ADR/learning surface stay hidden. Workaround: point the server at your repo explicitly with the global `--root` flag in your user config (`~/.claude/settings.json`):
>
> ```json
> { "mcpServers": { "orbit": { "command": "npx",
>   "args": ["-y", "@orbit-tools/cli@latest", "--root", "/abs/path/to/repo", "mcp", "serve"] } } }
> ```

---

## Agent Skills

`orbit workspace init` seeds skill files under `~/.orbit/skills/` and symlinks them into `~/.claude/skills/` and `~/.agents/skills/`, so Claude Code, Codex, and Gemini CLI discover them at session start with no per-agent configuration. The router skill (`orbit`) classifies intent; workflow-specific skills do the work:

- `orbit-task` — author a task, carry it through implementation and review, file findings on another agent's work, or capture agent-self-reported tooling friction
- `orbit-workflow` — use jobs, activities, routines, and `orbit sweep`/`orbit run`; diagnose failed, stuck, or cancelled runs
- `orbit-search` — search tasks, docs, learnings, and ADRs; dedup and related-task lookups; docs-corpus admin
- `orbit-knowledge` — author, accept, or supersede learnings and Architecture Decision Records
- `orbit-graph` — query the parsed code graph (refs, callees, impact, implementors)

First-time onboarding (`.orbit/` absent) and "what is orbit" tour requests are handled by the `orbit` skill itself, via its bundled setup reference.

`orbit skill doctor` flags drift between the local copy and the upstream definition. Edit any seeded `SKILL.md` to customize behavior for your team.

---

## Orbit MCP Tool Surface

`orbit workspace init --mcp` registers the Orbit MCP server with the local agent CLI (Claude Code, Codex, Gemini), same as the plugin. The table below is a tool reference; inactive or CLI/operator-only rows are called out separately from the active agent MCP surface. Run `orbit tool list` for the live registry (it's the source of truth; this table can drift).

Not every tool is intended for agent calls. Lifecycle/admin operations (`docs.index`, `docs.migrate`, `semantic.*`, `learning.sync`, `task.locks.*`, `friction.*` reads/updates, `graph.history`) are typically driven by humans via the CLI; the recommended agent permission profile auto-allows discovery/write tools and prompts on the rest. See `.claude/settings.json` (and `.codex/`, `.grok/`, `.gemini/` equivalents) in the seeded workspace for the default agent-facing subset.

<details>
<summary><strong>Full tool reference</strong> — task, review, graph, search, semantic, adr, docs, learning, friction (click to expand)
</summary>

Agents discover project docs through `orbit.search`; docs, lock, semantic setup/index/status, graph history, learning sync/list, and friction stats operations are CLI-only admin/setup workflows. Five further admin/destructive tools — `orbit.task.delete`, `orbit.task.lint`, `orbit.semantic.uninstall`, `orbit.adr.list` (use `orbit search --kind adr` from agents), `orbit.learning.prune` — remain registered for admin use via `orbit tool run` and the `orbit adr list` / `orbit task lint` CLI surfaces, but are hidden from the agent MCP surface (ORB-00289). ORB-10046 removed the learning vote and comment surfaces entirely — corrections go through `update`/`supersede`, provenance through `evidence`.

| Namespace | Tool | Purpose |
|---|---|---|
| **task** | `orbit.task.add` | Create a new task |
| | `orbit.task.update` | Mutate task fields (status, plan, acceptance criteria) |
| | `orbit.task.show` | Fetch full task detail |
| | `orbit.task.list` | List tasks filtered by status / scope / `path` |
| | `orbit.task.start` | Transition into in-progress |
| | `orbit.task.approve` | Approve a task (`proposed → backlog`, or `review → done`) |
| | `orbit.task.reject` | Reject a task |
| | `orbit.task.artifact.put` | Attach a generated artifact to a task |
| | `orbit.task.locks` | List files currently locked by active tasks |
| **review** | `orbit.task.review_thread.add` | Open a review thread on a task |
| | `orbit.task.review_thread.list` | List review threads on a task |
| | `orbit.task.review_thread.reply` | Reply to a thread |
| | `orbit.task.review_thread.resolve` | Close a thread |
| **graph** | `orbit.graph.search` | Find symbols / strings / config in the parsed graph |
| | `orbit.graph.show` | Show a node's source and metadata by selector |
| | `orbit.graph.overview` | Crate / module structural summary |
| | `orbit.graph.refs` | List inbound references / callers of a symbol |
| | `orbit.graph.callees` | List outbound calls from a symbol |
| | `orbit.graph.impact` | Bounded blast-radius traversal for a change |
| | `orbit.graph.trace` | Trace a command handler's call tree |
| | `orbit.graph.implementors` | List trait implementors |
| | `orbit.graph.deps` | List outbound module / import edges |
| | `orbit.graph.sync` | Refresh the index (auto-syncs on a watcher) |
| **search** | `orbit.search` | Unified search across tasks, docs, learnings, and ADRs. `kind` narrows the corpus; `hybrid: true` opts task results into BM25 + cosine ranking; `semantic: "<task-id>"` returns cosine neighbors. Cross-kind filters: `tag` (AND), `all` (kind-aware status widener), `status` (`kind:value` tokens), `path` (selector-mapping for tasks, glob-containment for learnings/ADRs; docs remain content-indexed). |
| **adr** | `orbit.adr.add` | Author an Architecture Decision Record |
| | `orbit.adr.update` | Edit an ADR |
| | `orbit.adr.show` | Fetch an ADR |
| | `orbit.adr.supersede` | Mark an ADR superseded by another |
| **docs** | `orbit.docs.list` | List indexed Markdown docs under configured `[docs].roots` |
| | `orbit.docs.show` | Show a single doc with parsed frontmatter and body |
| | `orbit.docs.add` | Register an additional docs root |
| **learning** | `orbit.learning.add` | Author a project learning |
| | `orbit.learning.update` | Edit a learning |
| | `orbit.learning.show` | Fetch a learning |
| | `orbit.learning.list` | List learnings by tag / scope / `path` (glob-containment) |
| | `orbit.learning.supersede` | Mark a learning superseded |
| **friction** | `orbit.friction.add` | Record an operational friction |
| | `orbit.friction.update` | Edit a friction |
| | `orbit.friction.show` | Fetch a friction |
| | `orbit.friction.list` | List frictions by tag / status |
| | `orbit.friction.tags` | List configured friction taxonomy tags |
| | `orbit.friction.resolve` | Mark a friction resolved |

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
├── graph/                       # parsed code-graph index (.db, per worktree)
├── resources/                   # activities, jobs, executors, policies (customizable)
└── state/
    ├── audit/                   # reserved; audit events live in ~/.orbit/orbit.db
    ├── job-runs/                # reserved; v2 run state lives in ~/.orbit/orbit.db
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

- Operating this state day-2 — backup/restore, stuck-job debugging, corrupted-DB recovery, log rotation, health checks (`orbit doctor`, `/healthz`), and upgrades (`orbit migrate`) — is covered in [docs/OPERATIONS.md](docs/OPERATIONS.md).

---

## Current Status

Orbit is v0.7.x — work in progress.

- Core local execution, graph build/query, workflows, MCP, tasks, reviews, ADRs, frictions, and audit infrastructure are usable today.

---

## Contributing

Contributions especially welcome on graph-aware scheduling, locking, worktree/session management, execution primitives, reconciliation, audit coverage, and tool-calling interfaces.

Before contributing: [docs/design/CONVENTIONS.md](docs/design/CONVENTIONS.md) and [CLAUDE.md](CLAUDE.md).

---

## License

MIT
