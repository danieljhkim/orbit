# Orbit Configuration

Reference for Orbit's runtime config — the `config.toml` consumed by `orbit run ship`, `duel-plan`, and the activity-job dispatcher. The defaults shipped with the binary live in [`crates/orbit-core/assets/config/default-config.toml`](../crates/orbit-core/assets/config/default-config.toml).

This doc focuses on the user-facing knobs: `[workflow]`, `[crews.*]`, and `[duel]`. Other sections are summarized at the end.

## Where config lives

Two paths are consulted, in order:

| Path | Scope | Created by |
|---|---|---|
| `<workspace>/.orbit/config.toml` | Workspace-local | Hand-authored (optional) |
| `~/.orbit/config.toml` | Global / user | `orbit init` |

**Workspace config REPLACES global config — it does not merge.** If `.orbit/config.toml` exists in your workspace, the global file is ignored entirely. This is intentional: per-repo agent behaviour (sandbox mode, approval policy, crew composition) must be fully deterministic and not silently inherit whatever happens to be in the user's global config.

The workspace identity file `.orbit/config.yaml` is a separate artifact (it stores `workspace_id` for the canonical task store binding) and is unrelated to runtime config.

---

## `[workflow]` — branch and crew defaults

```toml
[workflow]
base_branch = "main"        # default merge-base for ship / duel-plan
default_crew = "codex"      # fallback crew when a task has no `crew` set
```

- **`base_branch`** — the branch `orbit run ship` and `duel-plan` rebase against and target with PRs. Override per-invocation with `--base <branch>`. If your repo uses a two-branch pattern like this repo does (`main` = release, `agent-main` = dev integration), set `base_branch = "agent-main"`.
- **`default_crew`** — name of the crew under `[crews.<name>]` used for any task whose own `crew` field is unset. Must match a defined crew or config load fails. See [Per-task crew override](#per-task-crew-override) for how individual tasks select a different crew.

---

## `[crews.<name>]` — who runs which role

A **crew** assigns models to the three roles in the ship pipeline: `planner`, `implementer`, `reviewer`. Each role takes three fields:

| Field | Purpose | Values |
|---|---|---|
| `model` | Model identifier passed to the provider CLI | Provider-specific (e.g. `claude-opus-4-7`, `gpt-5.5`, `pro`, `grok-build`) |
| `provider` | Agent family | `claude`, `codex`, `gemini`, `grok` |
| `backend` | How Orbit dispatches the agent | `cli` (today the only supported value for these roles) |

Example — a single-family Codex crew:

```toml
[crews.codex]
planner     = { model = "gpt-5.5",         provider = "codex",  backend = "cli" }
implementer = { model = "gpt-5.5",         provider = "codex",  backend = "cli" }
reviewer    = { model = "gpt-5.5",         provider = "codex",  backend = "cli" }
```

You can define any number of crews. Set the workspace-wide fallback with `workflow.default_crew`; assign a specific crew to individual tasks via the [per-task crew override](#per-task-crew-override). Common patterns:

- **Single-family crew** (`claude`, `codex`, `gemini`, `grok`) — useful when you only have one CLI authenticated, or when you want consistent behaviour end-to-end.
- **Mixed crew** — different model for each role, e.g. a strong planner with a fast implementer, or a different vendor for review than implementation so the reviewer isn't reviewing its own output.

Crews are validated at load time: each role must have non-empty `model`, `provider`, and `backend`; `workflow.default_crew` must name a defined crew.

> **Note.** Earlier Orbit versions used `[agent.<role>]` tables. That schema was removed in [ORB-00058](../.orbit/) — config load now hard-errors if `[agent.*]` is present. Migrate to `[crews.<name>]` + `workflow.default_crew`.

---

## Per-task crew override

`[workflow].default_crew` is the workspace fallback, not a global verdict. **Every task carries an optional `crew` field**, and `orbit run ship` resolves which crew to dispatch per task in this order:

1. `task.crew` if set on the task artifact, otherwise
2. `[workflow].default_crew` from `config.toml`, otherwise
3. error: `no crew selected; set [workflow].default_crew, task.crew, or pass crew`.

This means you can mix-and-match in a single ship run: route a tricky refactor to `claude` while routing routine cleanups to `codex` — both go through the same `orbit run ship` invocation, each picking its own crew at dispatch time.

### Setting `task.crew`

Three equivalent surfaces:

| Surface | How |
|---|---|
| **Web dashboard** | The crew dropdown on each task card (the chevron next to `default: <crew>` in [`orbit web serve`](../README.md#quick-start)) — selecting a crew calls `orbit.task.update` under the hood. |
| **CLI** | `orbit task add --crew <name> …` at creation, or `orbit task update <id> --crew <name>` later. Pass `--crew ""` to `task update` to clear the field. (`orbit task start --crew <name>` exists but only validates the name and logs it — it does **not** persist onto `task.crew` or affect later `orbit run ship` dispatch. Use `task update` if you want the choice to stick.) |
| **MCP / agent** | `orbit.task.add` and `orbit.task.update` accept a `crew` parameter; an empty string on update clears it. Useful when an agent is filing or amending tasks programmatically. |

The dropdown label `default: codex` in the dashboard means *the task has no `crew` set* and will inherit `[workflow].default_crew`. Picking a named crew writes it onto the task and the label updates accordingly.

### What "ran" vs what "was selected"

`orbit.task.show` returns both fields when a run exists:

- `crew` — the task's own `crew` field (the *selection*).
- `resolved_crew` + `planner_model` / `implementer_model` / `reviewer_model` — what was actually dispatched (the *resolution*, including default-crew fallback). Pulled from the persisted job-run record so it stays accurate even if `default_crew` is edited later.

`task.crew` is validated at write time, so you can't `orbit task add --crew <name>` with an unknown crew. The only way to end up with a stale task-level override is to delete a crew from `config.toml` after it was already written onto tasks. In that case `orbit run ship` fails fast at run start — before any agent dispatches and before the `JobRunStarted` event is emitted — so no work is wasted.

---

## `[duel]` — bake-off candidates for `duel-plan`

`orbit duel-plan` runs the planning step across multiple agent families in parallel and scores the results. `[duel]` controls which families participate.

```toml
[duel]
candidates = ["codex", "claude", "gemini", "grok"]

[duel.models]
codex  = "gpt-5.5"
claude = "claude-opus-4-7"
gemini = "pro"
grok   = "grok-build"
```

- **`candidates`** — at least 3 distinct entries drawn from the valid family list (`codex`, `claude`, `gemini`, `grok`). Duplicates and unknown families are rejected at load.
- **`[duel.models]`** — optional per-family model override. Keys must be a subset of `candidates`. Values must be non-empty. When omitted, the duel executor uses a built-in model-pair default for that family.

Use `[duel]` to constrain which CLIs Orbit will spawn — e.g. drop `grok` from `candidates` if Grok Build isn't authenticated on this machine.

---

## Other sections (brief)

| Section | Purpose |
|---|---|
| `[execution.env]` | Env vars passed to agent subprocesses. `inherit = false` (default) means only the explicit `pass` list crosses the boundary; useful for keeping secrets out of agent CLIs. |
| `[execution.codex]` | Codex CLI sandbox mode. Valid: `read-only`, `workspace-write` (default), `danger-full-access`. Optional `approval_policy = "on-request"` enables escalation prompts. |
| `[task.approval]` | Whether agent-initiated tasks require human approval before execution (`required_for_agent`), and whether delegated subagent runs inherit that requirement (`delegate_approval`). |
| `[scoring]` | `enabled = true` records per-agent scoreboard counters under `.orbit/state/scoreboard/`. |
| `[graph]` | `editing = false` (default) makes the knowledge graph read-only from agent tools; flip to `true` to allow `orbit.graph.*` mutations. |
| `[pr]` | PR creation defaults (template, labels, draft mode) for `orbit run ship --mode pr`. |
| `[runtime]` | `backend = "cli" \| "http" \| "auto"` selects the activity-job dispatcher backend for v2 `agent_loop` activities. |

---

## Validation and errors

Config is parsed at startup; invalid entries fail loud rather than silently falling back. Common failure modes:

- `[duel] candidates must contain at least 3 entries` — duel requires a non-trivial bake-off.
- `[duel.models] contains key '<x>' that is not in resolved [duel].candidates` — model override for an unlisted family.
- `[workflow].default_crew = '<x>' is not defined under [crews]` — name a crew that exists.
- `config schema changed in ORB-00058; remove [agent.<role>] tables` — migrate to crews.
- `execution.codex.sandbox has invalid value` — must be `read-only`, `workspace-write`, or `danger-full-access`.

When in doubt, copy the default ([`crates/orbit-core/assets/config/default-config.toml`](../crates/orbit-core/assets/config/default-config.toml)) into `.orbit/config.toml` and edit from there.
