# Orbit — The engineering framework for your AI coding agents

<p align="center">
  <img src="docs/assets/orbit-dashboard-hero.gif" alt="Orbit dashboard: task backlog, agent execution, and live audit log" width="600" />
</p>

<p align="center">
  <em>The Orbit dashboard (<code>orbit web serve</code>) — task backlog, live audit log, per-agent scoreboard.</em>
</p>

**Orbit brings engineering rigor to AI-assisted coding. A durable task for every change, structured audit of every tool call and provider exchange, conflict-aware parallel dispatch, and a searchable corpus built from the docs you already write — local-first.**

You drive Claude Code, Codex, Grok Build, or Gemini CLI against real code, often in parallel. Agents make it easy to skip the disciplines that keep code maintainable — no plan, no record, no audit trail, just prompt-and-merge. Six months later you can't reconstruct why an agent wrote a given line. Orbit makes those disciplines cheap and enforces them by default: tasks before edits, every tool call landing in a structured audit log, parallel runs sandboxed into worktrees with file-level locks, and your team's own design docs retrievable by the agents doing the work.

The constraints are the point — they're what keep agent-assisted code shippable at volume. And because every commit carries its task ID, the history of how the code came to be stays reconstructable, by you and by the agents.

---

## Primary Features

- **Durable, intent-tracked task layer.** Lifecycle (`proposed → backlog → in-progress → review → done`) survives sessions and branches; every commit carries the `task_id`, so `orbit task show` reconstructs prompt, plan, execution trace, and review threads months later. → [docs/design/task-artifacts/](docs/design/task-artifacts/)

- **Structured audit log.** Every tool call, provider request/response, and task transition becomes a queryable event with agent identity attached — append-only, tamper-evident, exportable. → [docs/design/auditability/](docs/design/auditability/)

- **Conflict-aware parallel execution.** For `orbit run ship`, each agent run lands in its own git worktree per task, and the gate pipeline reserves task `context_files` as locks before fanning out, rejecting overlapping reservations up front instead of producing merge conflicts later (see [merge throughput chart](docs/assets/merge-throughput.png)). → [docs/design/activity-job/](docs/design/activity-job/)

- **Sandboxed-by-default execution.** Dispatched agent CLIs use a platform-specific OS boundary where supported. macOS uses `sandbox-exec`; Linux uses trusted `/usr/bin/bwrap` after a capability probe to enforce writes from the resolved policy. The Linux boundary leaves host filesystem reads and host network access available, so it does not provide worktree-only reads or policy-gated network egress. Windows and other unsupported platforms have no shipped OS-level backend, while in-process FS guards still cover HTTP tools. → [docs/design/policy-sandbox](docs/design/policy-sandbox/)

- **A searchable docs corpus — your conventions, not Orbit's.** Register the markdown you already write — designs, decision records, ADRs, runbooks, patterns, in whatever layout your team uses — with `orbit docs add`, and agents retrieve it by concept instead of by filename: `orbit search --kind doc <query>` ranks against locked frontmatter, and `orbit docs index` plus `--hybrid` adds body-level embedding recall. Orbit imposes no doc structure; it makes the structure you already have retrievable. → [docs/design/orbit-docs/](docs/design/orbit-docs/)

- **A friction ledger for what the tooling gets wrong.** When Orbit itself is the obstacle — a confusing error, a missing flag, a misleading prompt — the agent files it (`orbit friction add`, or `orbit_friction_add` over MCP) instead of silently working around it. Records are triaged (`open → triaged → resolved`) and a task carrying `relations: [{"type": "resolves", ...}]` closes its friction on reaching `done`.

- **Dependency-ordered execution.** Tasks carry `dependencies` and typed `relations` (`blocked_by`, …). The pipeline gates admission on them, so a dependent task waits for its blocker to reach `done` instead of being hand-sequenced by whoever is dispatching — declare the order once and let the queue enforce it.

- **Recurring work as data, not code.** `orbit auto-task add/list/show/update/toggle` defines `.orbit/auto_tasks/*.yaml` templates with a cron or interval schedule and a dedupe policy; a seeded scheduler routine mints tasks from the due ones, collapsing catch-up runs and skipping duplicates when one is already open. `orbit auto-task mint <name>` mints one on demand — same template mapping, same provenance — so a new definition can be exercised without waiting for its slot. Provider-neutral, checked into the repo.

---

## Quick Start

### Install the Binary — Recommended

One command gets you a released, signed build. Within your first fifteen minutes you have durable tasks, an audit log, MCP tools registered with your agent CLI, and the dashboard — no clone, no Rust toolchain.

**Prerequisites:** at least one supported agent CLI (Codex, Claude Code, Cursor, or Gemini CLI), authenticated. For PR-based workflows (i.e., `orbit run ship` in the default `--mode pr`), `gh` installed and authenticated; otherwise use `--mode local`. On Linux, install and verify the [Linux Bubblewrap host prerequisite](#linux-bubblewrap-host-prerequisite) after `orbit init`.

```bash
# install
curl -sSf https://raw.githubusercontent.com/danieljhkim/orbit/main/install.sh | sh
# or: brew install danieljhkim/tap/orbit
```

<details>
<summary><strong>Your first fifteen minutes</strong> — initialize, ship a task, watch it land (click to expand)</summary>

```bash
# initialize
orbit init                                 # global state (~/.orbit)
cd <repo> && orbit workspace init --mcp    # workspace state + operator-authorized MCP integration

# create, approve, and ship a task
TASK_ID=$(orbit task add \
  --title "..." \
  --description "..." \
  --acceptance-criteria "..." \
  --complexity medium \
  --workspace .)

# or simply ask an agent to create a task:
# "Claude can you create an orbit task to refactor the authentication logic in ..."

orbit task update "$TASK_ID" --status backlog   # approve into the backlog

# conflict-aware, parallel flush of the backlog tasks to PRs.
# jobs are asynchronous: this submits and returns a run id immediately.
orbit run ship
orbit run history -j task_auto_pipeline   # what got submitted
orbit run show <RUN_ID>                   # step-by-step progress

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

Everything is incremental from here: the task layer and audit log work on day one, and the docs corpus, friction ledger, auto-tasks, and parallel dispatch switch on as you adopt them — none is a prerequisite for the others.

### Clone & Customize via Agent Prompt

The binary is a released build; cloning gives you a customizable framework to mold into your team's conventions. Everything `.orbit/` holds carries over, so you can start on the binary and switch later. No need to contribute back to Orbit unless you want to, you can just fork it.

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
> Your job is to install and configure Orbit inside this repository so that I can keep using my existing agents while gaining durable tasks, structured audit, a searchable docs corpus, and safe parallel execution.
>
> Follow these steps carefully:
>
> 1. Ask me where I want to clone the Orbit repository (suggest something like `~/code/orbit` or `~/dev/orbit`).
> 2. Verify the Rust toolchain. Run `cargo --version` and `rustc --version`. Orbit declares `rust-version = "1.89"` (MSRV), so I need Rust **1.89 or newer**. If cargo is missing, or rustc is older than 1.89, **stop and ask me before installing anything** — the canonical path is `rustup` (`curl https://sh.rustup.rs | sh`), but that modifies shell profile, so I want to confirm first. If rustup is already installed but the toolchain is old, suggest `rustup update stable` and confirm before running.
> 3. Clone `https://github.com/danieljhkim/orbit` into the location from step 1, then run `make install`. This builds with cargo and copies the `orbit` binary to `$INSTALL_BIN_DIR` (default: `~/.cargo/bin`). Confirm the install path with me before running. Verify with `orbit --version`.
> 4. Run `orbit init` to initialize global state at `~/.orbit`. It selects the host-appropriate sandbox for the shipped executor definitions: `macos-sandbox-exec` on macOS, `linux-bwrap` on Linux, and no OS-level backend on unsupported platforms. On Linux, after `orbit init`, follow the [Linux Bubblewrap host prerequisite](#linux-bubblewrap-host-prerequisite) below and require its capability probe to pass before dispatching agents.
> 5. From *this* repository (not the Orbit clone), run `orbit workspace init --mcp`. This creates `.orbit/` here and auto-registers Orbit's MCP server with installed agent CLIs (Claude Code, Codex, Gemini). The registered server is **operator-authorized**: it can dispatch workflows (`orbit.workflow.ship`, run observation/resume) and run `orbit.command.exec`. Tell me before running this if you'd rather it stay agent-only (`orbit mcp init` instead).
> 6. Ask me whether to enable semantic search (**optional**). `orbit semantic install` downloads a small embedder companion plus the default bge-small model (lives under `~/.orbit/embed/`) and powers `orbit search <query> --hybrid` / `orbit search similar <task-id>` over tasks. It requires macOS arm64 or Linux x86_64/aarch64 with glibc >= 2.38; Intel macOS is unsupported for semantic search. Don't install without my OK. If I accept and tasks already exist in this workspace, also run `orbit semantic index` to backfill the corpus.
> 7. Read the key documents so you actually understand the model:
>    - `README.md` — feature surface and install model
>    - `docs/POSITIONING.md` — what Orbit is for, what it isn't (especially "who this is for")
>    - `CLAUDE.md` — agent operating rules (commit timing, task ID convention, lint constraints)
>    - `ARCHITECTURE.md` — crate layering and dependency rules
>    - `docs/design/CONVENTIONS.md` — design-doc structure, and the admission test for what earns a decision entry
>    - `docs/CONFIG.md` — config reference: crew/workflow knobs and per-task crew override
> 8. After setup, run `orbit task list` and `orbit semantic stats` and show me the output.
> 9. Ask me what my first real task should be and create it properly using Orbit's task surface (use the `orbit` skill — it should be auto-discovered after step 5).
>
> Rules:
> - Never run destructive commands without explicit confirmation. Specifically: cloning, installing rustup, running `make install` outside `~/.cargo/bin`, and any shell-profile modification all need a confirmation prompt.
> - If anything is unclear or fails, stop and ask me.
> - Do not try to "make it simpler" or hide Orbit's conventions. I am choosing this because I want the discipline.
>
> Report back what you did and the current state of the workspace.

</details>

### Linux Bubblewrap host prerequisite

`orbit init` persists the host-appropriate sandbox into the shipped executor artifacts. On Linux that value is `linux-bwrap`, which resolves the trusted wrapper at `/usr/bin/bwrap` and fails closed if its namespace-and-mount capability probe cannot run. Install Bubblewrap before dispatching an agent. Ubuntu 24.04 (Noble) also enables AppArmor restrictions on unprivileged user namespaces; without the distro's narrow Bubblewrap profile, the probe fails with `bwrap: setting up uid map: Permission denied`.

On Ubuntu 24.04, run this remediation and verification sequence. The package ships `bwrap-userns-restrict` under `/usr/share/apparmor/extra-profiles/`; copy that narrow profile into `/etc/apparmor.d/`, load it, confirm that AppArmor knows it, and run the same capability shape Orbit probes:

```bash
sudo apt-get update
sudo apt-get install --yes bubblewrap apparmor-profiles
test -x /usr/bin/bwrap
test -f /usr/share/apparmor/extra-profiles/bwrap-userns-restrict
sudo install -m 0644 \
  /usr/share/apparmor/extra-profiles/bwrap-userns-restrict \
  /etc/apparmor.d/bwrap-userns-restrict
test -f /etc/apparmor.d/bwrap-userns-restrict
sudo apparmor_parser -r /etc/apparmor.d/bwrap-userns-restrict
grep -Fq 'bwrap-userns-restrict' /sys/kernel/security/apparmor/profiles

/usr/bin/bwrap \
  --die-with-parent \
  --new-session \
  --unshare-all \
  --share-net \
  --ro-bind / / \
  -- /bin/true
```

The final command must exit successfully. If it still reports the UID-map error, do not disable `kernel.apparmor_restrict_unprivileged_userns` globally and do not enable `allow_fallback`; both weaken or bypass the fail-closed boundary. Re-check the packaged profile path and load output, then rerun the probe. Non-Linux setup paths are unchanged.

Full command reference: `orbit --help` and [orbit-cli.com](https://orbit-cli.com).

Customizing crews (which provider-model runs a task) and the base branch: see [docs/CONFIG.md](docs/CONFIG.md).

---

## Semantic Search (optional)

`orbit search` is the unified query surface over three corpora — `--kind task`, `--kind doc`, `--kind friction`, or `--kind all`. It defaults to lexical matching. Opt into hybrid embedding ranking over task fields or indexed docs with `--hybrid`, or find cosine-neighbor tasks with `orbit search similar <task-id>`. The embedder runs as a separate companion subprocess, so semantic search has zero cost when unused.

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

After install, task writes are embedded automatically in the background; `orbit semantic index` is only needed for the initial task backfill. Docs are indexed explicitly with `orbit docs index`, which skips unchanged content hashes and sweeps stale doc paths. Companion + models live under `~/.orbit/embed/`; the per-workspace index is `.orbit/state/semantic.db`. Only tasks and docs are embedded — friction results stay lexical even when `--hybrid` is set.

---

> **Cowork users:** Orbit advertises its canonical MCP surface independently of the
> server's launch directory. Workspace routing comes from the server's
> `--workspace` binding, MCP initialize/session context, or an explicit registered
> `workspace` on the tool call — never from cwd. Do not add the global `--root`
> flag to `orbit mcp serve`; the server rejects launch-root routing so a
> scratchpad cwd cannot silently select the wrong workspace.

---

## Agent Skills

`orbit workspace init` seeds skill files under `~/.orbit/skills/` and symlinks them into `~/.claude/skills/` and `~/.agents/skills/`, so Claude Code, Codex, and Gemini CLI discover them at session start with no per-agent configuration. The same pass removes dangling Orbit-owned links for skill IDs that left the default set (a leftover custom skill directory or a live custom symlink is left alone).

Orbit ships one skill, `orbit`. Its `SKILL.md` is a router — the tool-invocation surface, the lifecycle, and a table of references that load on demand:

- **Working through Orbit** — task authoring, execution, and review; search and the docs corpus; friction; orchestration and the `orbit run` surface; debugging a failed run.
- **Setting Orbit up** — first run and host identity, configuration and crews, the routine scheduler, auto-tasks, maintenance and worktree GC, multi-host task-ID namespacing, and remote access.

First-time onboarding (`.orbit/` absent) and "what is orbit" tour requests are handled by the same skill, via its bundled setup references.

`orbit skill doctor` flags drift between the local copy and the upstream definition, and reports dangling or orphaned symlinks under the client skill link directories. Edit any seeded `SKILL.md` to customize behavior for your team.

---

## Orbit MCP Surface

`orbit workspace init --mcp` registers the Orbit MCP server with the local agent CLI (Claude Code, Codex, Gemini). The registered server launches as `orbit mcp serve --operator --workspace <ws_id>`: it holds **operator authority**, so governed operations — dispatching a workflow (`orbit.workflow.ship`), observing/resuming a run, and `orbit.command.exec` — are authorized through it. Bare `orbit mcp serve` (and every worker/agent-launched MCP session) stays agent-only and is refused those governed tools; `orbit mcp init` also stays agent-only unless you pass `--operator` at the `orbit mcp serve` layer yourself.

`--workspace` is the workspace binding: most MCP clients cannot announce one at initialize, so the generated integration names the workspace it was registered for and workspace-scoped tools route without repeating a selector on every call. An explicit `workspace` on a tool call still overrides it, and a server launched without a binding still refuses a workspace-scoped call that names no workspace.

`tools/list` is the authoritative MCP reference. Orbit composes that surface from
the explicitly MCP-registered builtins in `orbit-tools` plus the two discovery
tools `orbit.workspace.list` and `orbit.crew.list`. `orbit tool list` remains the
broader registry reference, including tools deliberately kept off MCP.

Every advertised tool is visible to every session regardless of authority tier;
authorization is enforced at call time, not by hiding names from `tools/list`.
Local and SSH-originated sessions receive the same complete supported surface.
Client permission profiles may auto-allow some names and prompt for others, but
those settings are ergonomics, not a server security boundary. Domain
validation, sandboxing, workspace claims, and other runtime checks still
execute on the accepting server through Orbit Core.

Workspace-scoped tools accept a registered workspace name, logical `ws_*` ID, or
absolute path registered on the accepting server. The selector may come from the
tool's `workspace` argument or MCP initialize metadata; server process cwd is not
a fallback. `orbit.workspace.list` is global and reports active workspaces that
have a checkout registered on that machine.

Remote MCP uses one direct, byte-transparent SSH stdio hop:

```text
orbit mcp serve --mode remote <ssh-host>
  -> ssh -T <ssh-host> orbit mcp serve --remote-caller-machine-id <audit-label>
```

SSH access is sufficient for v1. Caller labels and the best-effort SSH source IP
are audit metadata only; future authorization belongs in Core.

---

## Workspace Layout of `.orbit`

- `orbit workspace init` creates a `.orbit/` directory at the repo root. Workspace-local state lives there; removing the directory returns the workspace to a pre-init state.
- `orbit init` creates a `.orbit/` directory in the user's home (`~/.orbit/`). User-scoped state lives there; removing the directory returns the user environment to a pre-init state.

```
.orbit/                          # workspace-local (safe to delete → clean slate)
├── config.toml                  # workspace-local runtime overrides
├── config.yaml                  # workspace_id only
├── auto_tasks/                  # recurring task definitions
├── routines/                    # routine definitions
├── tasks/                       # symlinks → ~/.orbit/tasks/workspaces/<id>/
├── resources/                   # activities, jobs, executors, policies (customizable)
└── state/
    ├── worktrees/               # live git worktrees for agent runs
    ├── logs/                    # captured agent stdout/stderr
    ├── job-runs/                # per-run step state and checkpoints
    ├── audit/                   # workspace-local audit spool
    ├── diagnostics/             # doctor / health-check output
    ├── scoreboard/              # rolling counters (PRs, runs, etc.)
    ├── semantic.db              # embedding index for tasks and docs
    └── layout.version           # layout migration marker

~/.orbit/                        # global (machine-level, survives repo moves)
├── tasks/
│   ├── index.sqlite             # authority for ORB-XXXXX IDs
│   └── workspaces/<workspace-id>/<task-id>/   # canonical task bundles
├── orbit.db                     # host-global store: audit events, job runs,
│                                #   routines, frictions
├── workspaces.json              # workspace registry for this machine
├── resources/                   # shipped activities, jobs, executors, policies
├── skills/                      # SKILL.md files (routable via MCP)
├── embed/                       # semantic companion binary + models
├── config.toml                  # global settings
└── host.toml                    # machine identity: machine_id, host_id,
                                 #   task_prefix
```

Couple things to note:
- **`tasks/`** is a projection. Canonical task bundles live under `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/` so they survive repo moves; `.orbit/tasks/` is rebuildable from the canonical store. See [docs/design/task-artifacts/](docs/design/task-artifacts/).

- Global state — credentials, the canonical task store, and cross-workspace config — lives under `~/.orbit/`, created by `orbit init`. The recommended `.gitignore` pattern is `.orbit/*` with `!.orbit/auto_tasks/`, `!.orbit/resources/`, `!.orbit/routines/`, and `!.orbit/config.toml` un-ignored, so local runtime state stays out of the repo while the definitions your team authors stay in. `orbit workspace init` seeds this pattern.

- Operating this state day-2 — backup/restore, stuck-job debugging, corrupted-DB recovery, log rotation, health checks (`orbit doctor`, `/healthz`), and upgrades (`orbit migrate`) — is covered in the [runbook index](docs/INDEX.md#runbooks).

---

## Current Status

Pre-1.0 and under active development. Breaking changes ride a minor bump (`0.10.x → 0.11.0`); see [CHANGELOG.md](CHANGELOG.md) and [RELEASING.md](RELEASING.md).

- Core local execution, workflows, MCP, tasks, docs, frictions, and audit infrastructure are usable today.
- 0.11.0 removed two native knowledge stores: `orbit adr` / `orbit.adr.*` and
  `orbit learning` / `orbit.learning.*` are gone. Durable know-how now lives in
  your own markdown registered into the docs corpus, in tasks, or in friction
  records — Orbit no longer prescribes a decision-record format. Workspaces
  migrate on open.
- The former parsed code-graph subsystem is also gone. Agents inspect source
  with `grep`/`rg` and direct file reads.

---

## Contributing

Contributions especially welcome on locking, worktree/session management, execution primitives, reconciliation, audit coverage, and tool-calling interfaces.

Before contributing: [docs/INDEX.md](docs/INDEX.md#designs),
[docs/design/CONVENTIONS.md](docs/design/CONVENTIONS.md), and [CLAUDE.md](CLAUDE.md).

---

## License

MIT
