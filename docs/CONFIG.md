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
default_crew = "sol"        # fallback crew when a task has no `crew` set
```

- **`base_branch`** — the branch `orbit run ship` and `duel-plan` rebase against and target with PRs. Override per-invocation with `--base <branch>`. If your repo uses a two-branch pattern like this repo does (`main` = release, `agent-main` = dev integration), set `base_branch = "agent-main"`.
- **`default_crew`** — name of the crew under `[crews.<name>]` used for any task whose own `crew` field is unset. Must match a defined crew or config load fails. See [Per-task crew override](#per-task-crew-override) for how individual tasks select a different crew.

---

## `[crews.<name>]` — which provider-model runs the task

A **crew** is one provider-model-backend assignment. Every activity role (`planner`, `implementer`, or `reviewer`) resolves to that same assignment; the role remains a prompt and telemetry label, not a separate model-selection slot.

| Field | Purpose | Values |
|---|---|---|
| `model` | Model identifier passed to the provider CLI | Provider-specific (e.g. `opus`, `sonnet`, `gpt-5.6-sol`, `pro`, `grok-build`) |
| `provider` | Agent family | `claude`, `codex`, `gemini`, `grok` (the CLI-executable families; see [Provider identity and resolution](#provider-identity-and-resolution) for the full canonical set) |
| `backend` | How Orbit dispatches the agent | `cli` (today the only supported value for these roles) |
| `description` | Optional human-facing crew summary | Any non-empty string after trimming |
| `tags` | Optional discovery labels | Array of strings; normalized, sorted, and deduplicated |

Example — the standard Codex Sol crew:

```toml
[crews.sol]
model = "gpt-5.6-sol"
provider = "codex"
backend = "cli"
description = "Systems implementation"
tags = ["implementation", "review"]
```

Fresh `orbit init` configuration advertises only detected provider CLIs. Claude seeds `opus`, `sonnet`, and `fable`; Codex seeds `sol`, `terra`, and `luna`; Gemini seeds `gemini`; and Grok seeds `grok`. When Codex or Claude is available, `qa` uses Terra or Sonnet respectively. Every generated entry uses the CLI backend. If no supported provider CLI is detected, init leaves both the crew registry and `workflow.default_crew` unset instead of writing an unusable provider.

You can define any number of crews. Set the workspace-wide fallback with `workflow.default_crew`; assign a specific crew to individual tasks via the [per-task crew override](#per-task-crew-override). Crews are validated at load time: each crew must have non-empty `model`, `provider`, and `backend`; `workflow.default_crew` must name a defined crew.

Crew metadata is canonical runtime data, not display-only TOML. Orbit trims
`description` (blank becomes absent), trims each tag, drops blank tags, and stores
tags in sorted deduplicated order. Legacy crew entries without these fields
normalize to no description and an empty tag list. Owner execution-profile
publication carries this complete projection to the hub; publication is stricter
than legacy dispatch compatibility and fails closed if provider or backend cannot
be canonicalized to a concrete executable combination.

> **Legacy compatibility.** Orbit still accepts the former `planner` / `implementer` / `reviewer` inline-table shape. It uses the `implementer` assignment for every role and logs an `orbit.config.crew` warning when planner or reviewer differs. Rewrite legacy crews to the flat shape above; cross-provider comparison belongs in the duel system.

> **Note.** Earlier Orbit versions used `[agent.<role>]` tables. That schema was removed in [ORB-00058](../.orbit/) — config load now hard-errors if `[agent.*]` is present. Migrate to `[crews.<name>]` + `workflow.default_crew`.

---

## Provider identity and resolution

Every `provider` string Orbit reads — in `[crews.<name>]`, in an activity's inline `provider`, and in setup detection — is parsed through **one canonical surface** (`orbit_common::types::activity_job::Provider`, ORB-10091). Centralizing parsing means the crew layer, the agent-role resolver, the CLI executor, and reconciliation cannot disagree with each other or with Worker/Bridge about what a provider name means.

### Canonical providers

| Canonical id | Aliases | CLI runtime | HTTP transport | Worker-executable |
|---|---|---|---|---|
| `claude` | — | yes | yes | yes |
| `codex` | — | yes | no | yes |
| `gemini` | — | yes | no | yes |
| `grok` | — | yes | no | yes |
| `ollama` | — | **unsupported at the Orbit CLI entry point** | no | **no** |
| `openai_compat` | `openai-compat` | **no** (HTTP-only) | no | **no** |

- **Parsing is case- and whitespace-insensitive.** `Claude`, `  claude `, and `CLAUDE` all resolve to `claude`. `openai-compat` normalizes to `openai_compat`.
- **Deprecated aliases resolve *and* warn.** The legacy vendor names normalize to their canonical id and log an `orbit.config.crew` deprecation warning (`{alias, canonical}`) — they never fail, but update the config:

  | Deprecated alias | Canonical |
  |---|---|
  | `anthropic` | `claude` |
  | `openai`, `chatgpt` | `codex` |
  | `google` | `gemini` |
  | `xai` | `grok` |

- **Canonical ≠ Worker-executable.** Orbit's canonical set is deliberately wider than what the model-neutral Worker leaf executor can run: `ollama` and `openai_compat` are first-class Orbit providers but Worker does not execute them. This distinction is preserved on purpose — do not narrow the canonical set to Worker's subset.
- **Known ≠ executable at this entry point.** The shared contract recognizes `ollama`, but the Orbit CLI capability set is the canonical cross-repo four; explicitly selecting `ollama` fails as `provider.unsupported` rather than falling back.
- **`openai_compat` is HTTP-only.** It has no CLI runtime, so selecting it with `backend = "cli"` fails structurally (see below) rather than falling back.

### Resolution precedence

Provider selection is **two composed steps**, not one. Describe them precisely — the inline `provider` on an activity is the *template baseline*, **not** an explicit override that outranks the crew.

**1 — Which crew is dispatched** (`resolve_crew_for_task`). The crew *name* is chosen by the Constellation provider-resolution precedence (contract §3), first non-empty tier wins:

1. **explicit** — `--crew` flag / run-input `crew`.
2. **task_config** — `task.crew` on the task artifact.
3. **workspace_default** — `[workflow].default_crew` in `config.toml`.
4. **environment_default** — the `CONSTELLATION_DEFAULT_PROVIDER` environment variable (a canonical provider id, which names the same-named single-family crew).
5. **system_default** — the canonical baseline (see below).

**2 — The selected crew's assignment overrides the activity's inline baseline** (`resolve_from_config`). For each `(provider, model, backend)` field independently: the selected crew value wins **when present**; otherwise the activity's inline `agent_loop` value stands. A crew override that omits a field (or whose `provider` string is unparseable) leaves the inline baseline in place — so a config typo never coerces dispatch onto a wrong runtime. This is also why **persisted provider identity is never re-defaulted** during reconciliation: a provider already frozen on a run record is reused verbatim, not reset to the enum default.

### The one setting that changes the default — `CONSTELLATION_DEFAULT_PROVIDER`

`CONSTELLATION_DEFAULT_PROVIDER` occupies the **environment_default** tier (4) — below any explicit / task / workspace choice, above the persisted baseline. Setting it to a canonical id (or a deprecated alias, which normalizes) re-defaults **every otherwise-defaulted resolution path at once**, without editing any repo or per-workspace config; a path that already made a higher-precedence choice is deliberately unaffected. Because Orbit seeds `[workflow].default_crew` on `orbit init`, a normally-configured workspace resolves at the workspace tier, so the env lever governs paths that reach resolution without a configured crew and **never overrides a configured `[workflow].default_crew`**.

> **System default.** The canonical Constellation system default is `claude`. When no higher tier selects a crew, Orbit dispatches the same-named `claude` crew. Existing workspaces whose `[workflow].default_crew` is `codex` retain that higher-precedence configured choice; the system fallback does not rewrite workspace configuration.

### No silent fallback

Explicit selections that are unsupported or unavailable **fail with a stable diagnostic and never fall back** to a different runtime:

- `provider openai_compat is unsupported by the Orbit CLI entry point (HTTP-only)` — a CLI-executable dispatch selected an HTTP-only provider.
- `provider ollama is unsupported by the Orbit CLI entry point` — a known provider is outside this entry point's capability set.
- `unknown provider '<x>'; expected one of claude, codex, gemini, grok, ollama, openai_compat — no CLI runtime registered` — the provider string did not resolve to a canonical id.

An **unrecognized `[crews.<name>].provider` value** is the one non-fatal case: it is logged (`orbit.config.crew` warn) and that field falls back to the activity's inline `provider`, because a config typo should not coerce dispatch onto a wrong runtime — the inline value is the known-good identity, not a default guess.

---

## Per-task crew override

`[workflow].default_crew` is the workspace fallback, not a global verdict. **Every task carries an optional `crew` field**, and `orbit run ship` resolves which crew to dispatch per task by the [resolution precedence](#resolution-precedence) above:

1. explicit `--crew` / run-input `crew`, otherwise
2. `task.crew` if set on the task artifact, otherwise
3. `[workflow].default_crew` from `config.toml`, otherwise
4. `CONSTELLATION_DEFAULT_PROVIDER` if set (environment tier), otherwise
5. the canonical `claude` system-default crew.

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
- `resolved_crew` + `crew_model` — what was actually dispatched (the *resolution*, including default-crew fallback). Pulled from the persisted job-run record so it stays accurate even if `default_crew` is edited later.

`task.crew` is validated at write time, so you can't `orbit task add --crew <name>` with an unknown crew. The only way to end up with a stale task-level override is to delete a crew from `config.toml` after it was already written onto tasks. In that case `orbit run ship` fails fast at run start — before any agent dispatches and before the `JobRunStarted` event is emitted — so no work is wasted.

---

## `[duel]` — bake-off candidates for `duel-plan`

`orbit duel-plan` runs the planning step across multiple agent families in parallel and scores the results. `[duel]` controls which families participate.

```toml
[duel]
candidates = ["codex", "claude", "gemini", "grok"]

[duel.models]
codex  = "gpt-5.5"
claude = "opus"
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
| `[tasks]` | `id_start = N` sets a floor for the local task-id allocator: on runtime build the counter is raised to at least `N` (never lowered), so machines can hold disjoint id ranges (e.g. one `0–9999`, another `10000+`) and avoid cross-machine collisions. Capped by `ORB_TASK_ID_MAX` (99999) — setting it near the ceiling shrinks the usable range. Prefer the one-shot `orbit workspace init --task-id-start N` for the initial seed; the config key keeps the floor sticky across machines that share a config. See [task-migration overview](design/task-migration/1_overview.md). |
| `[scoring]` | `enabled = true` records per-agent scoreboard counters under `.orbit/state/scoreboard/`. |
| `[graph]` | `editing = false` (default) makes the knowledge graph read-only from agent tools; flip to `true` to allow `orbit.graph.*` mutations. |
| `[pr]` | PR creation defaults (template, labels, draft mode) for `orbit run ship --mode pr`. |
| `[runtime]` | `backend = "cli" \| "http" \| "auto"` selects the activity-job dispatcher backend for v2 `agent_loop` activities. **JSONL log rotation/retention** (`~/.orbit/state/logs/orbit.jsonl`): `log_retention_days` (default `7`) deletes archives older than N days; `log_max_total_mb` (default `500`) caps total archive size, pruning oldest first; `log_max_file_mb` (default `100`) rolls the active file to a dated archive once it exceeds N MiB. Rotation runs opportunistically at process start. Invalid values (`0`, or `log_max_file_mb > log_max_total_mb`) are rejected at config load. |

---

## Validation and errors

Config is parsed at startup; invalid entries fail loud rather than silently falling back. Common failure modes:

- `[duel] candidates must contain at least 3 entries` — duel requires a non-trivial bake-off.
- `[duel.models] contains key '<x>' that is not in resolved [duel].candidates` — model override for an unlisted family.
- `[workflow].default_crew = '<x>' is not defined under [crews]` — name a crew that exists.
- `config schema changed in ORB-00058; remove [agent.<role>] tables` — migrate to crews.
- `execution.codex.sandbox has invalid value` — must be `read-only`, `workspace-write`, or `danger-full-access`.
- `tasks.id_start N exceeds maximum task id 99999` — the allocator start must fit the `ORB-00000` id space.
- `tasks.id_start N would lower the allocator below its current position M` — the counter only moves forward (raised only via `orbit workspace init --task-id-start`; the config key is a silent forward-only floor).

When in doubt, copy the default ([`crates/orbit-core/assets/config/default-config.toml`](../crates/orbit-core/assets/config/default-config.toml)) into `.orbit/config.toml` and edit from there.
