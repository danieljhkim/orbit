---
type: context
summary: Orbit Configuration
last_validated: 2026-08-15
---

# Orbit Configuration

Reference for Orbit's runtime config — the `config.toml` consumed by `orbit run ship` and the activity-job dispatcher. The defaults shipped with the binary live in [`crates/orbit-core/assets/config/default-config.toml`](../crates/orbit-core/assets/config/default-config.toml).

This doc focuses on the user-facing knobs: `[workflow]` and `[crews.*]`. Other sections are summarized at the end.

## Where config lives

Two paths are consulted, in order:

| Path | Scope | Created by |
|---|---|---|
| `<workspace>/.orbit/config.toml` | Workspace-local | Hand-authored (optional) |
| `~/.orbit/config.toml` | Global / user | `orbit init` |

Ordinary settings inherit per key: workspace values override global values, global values fill omissions, and built-in defaults fill remaining gaps.

Tables layer down to individual settings, while scalar and array values replace the matching global value. Named crews layer by crew name and field, so this is a complete workspace override when the global file already defines `sol`:

```toml
[crews.sol]
model = "gpt-5.6-terra"
```

Three security-sensitive settings deliberately do not inherit from global whenever a distinct workspace file exists:

- `execution.codex.sandbox`
- `execution.codex.approval_policy`
- `execution.env.pass`

If the workspace file omits one of these, Orbit uses that setting's built-in default. This keeps repository agent sandboxing, approval, and environment passthrough deterministic instead of depending on a user's global policy. `execution.env.inherit` is not a configurable key: agent subprocesses always start from a cleared environment.

Run `orbit config show` for the effective merged view. Every setting is annotated as `workspace`, `global`, `built-in`, or `environment`, including the source file path where one applies. `orbit config show --json` exposes the same attribution in its `provenance` object. Use `--scope global` or `--scope workspace` to inspect either physical file alone.

The workspace identity file `.orbit/config.yaml` is a separate artifact (it stores `workspace_id` for the canonical task store binding) and is unrelated to runtime config.

---

## `[workflow]` — branch and crew defaults

```toml
[workflow]
base_branch = "main"        # default merge-base for ship
default_crew = "sol"        # fallback crew when a task has no `crew` set
system_crew = "qa"           # recovery and failed-run-triage crew
```

- **`base_branch`** — the branch `orbit run ship` rebases against and targets with PRs. Override per-invocation with `--base <branch>`. If your repo uses a two-branch pattern like this repo does (`main` = release, `agent-main` = dev integration), set `base_branch = "agent-main"`.
- **`default_crew`** — name of the crew under `[crews.<name>]` used for any task whose own `crew` field is unset. Must match a defined crew or config load fails. See [Per-task crew override](#per-task-crew-override) for how individual tasks select a different crew.
- **`system_crew`** — name of the crew for `step_failure_recovery` and `triage_failed_runs`; defaults to `qa`, which `orbit init` seeds when it can configure an agent. It is resolved at every dispatch through the activities' explicit crew input, so it does not inherit a failed task's crew or the workspace default. A missing or unusable crew leaves the original failed step failed and emits a diagnostic naming `workflow.system_crew` and the configured crew.

---

## `[crews.<name>]` — which provider-model runs the task

A **crew** is one provider-model assignment. Activities do not carry a model-selection role: a rendered activity input may name a `crew`, and otherwise the activity inherits the run's resolved crew.

| Field | Purpose | Values |
|---|---|---|
| `model` | Model identifier passed to the provider CLI | Provider-specific (e.g. `opus`, `sonnet`, `gpt-5.6-sol`, `pro`, `grok-4.6`) |
| `provider` | Agent family | `claude`, `codex`, `gemini`, `grok` (the CLI-executable families; see [Provider identity and resolution](#provider-identity-and-resolution) for the full canonical set) |
| `description` | Optional human-facing crew summary | Any non-empty string after trimming |
| `tags` | Optional discovery labels | Array of strings; normalized, sorted, and deduplicated |

Example — the standard Codex Sol crew:

```toml
[crews.sol]
model = "gpt-5.6-sol"
provider = "codex"
description = "Systems implementation"
tags = ["implementation", "review"]
```

Example — the standard Grok crew:

```toml
[crews.grok]
model = "grok-4.6"
provider = "grok"
```

The current Grok Build CLI lists `grok-4.6` as its default from `grok models`, so Orbit uses that live menu id. The older `grok-build` string is not retained as a default or alias.

Fresh `orbit init` configuration advertises only detected provider CLIs. Claude seeds `opus`, `sonnet`, and `fable`; Codex seeds `sol`, `terra`, and `luna`; Gemini seeds `gemini`; and Grok seeds `grok`. When Codex or Claude is available, `qa` uses Terra or Sonnet respectively. If no supported provider CLI is detected, init leaves both the crew registry and `workflow.default_crew` unset instead of writing an unusable provider.

You can define any number of crews. Set the workspace-wide fallback with `workflow.default_crew`; assign a specific crew to individual tasks via the [per-task crew override](#per-task-crew-override). Crews are validated at load time: each crew must have non-empty `model` and `provider`; `workflow.default_crew` must name a defined crew.

Crew metadata is runtime data, not display-only TOML. Orbit trims `description`
(blank becomes absent), trims each tag, drops blank tags, and stores tags in sorted
deduplicated order. `orbit.crew.list` reads and normalizes the selected checkout's
effective local configuration on the machine serving the request. It returns the
versioned `CrewDiscoveryV1` projection directly; no execution-profile publication or
registry database is involved.

> **Retired crew shape.** `planner`, `implementer`, and `reviewer` sub-tables
> are no longer accepted in a crew entry. A workspace using that old shape must
> rewrite every `[crews.<name>]` entry to set flat `model` and `provider`
> fields before Orbit can load its configuration. Use separate crew-bound runs
> when comparing providers.

> **Retired `backend` field.** `[crews.<name>] backend` selected the agent
> execution backend. Orbit executes agent activities through the CLI agent path
> only, so the setting no longer chooses anything: `backend = "cli"` is accepted
> and ignored, while `"http"` and `"auto"` are rejected at config load with the
> migration message. Remove the key. Orbit never rewrites `http` to the CLI
> agent for you — that would change which runtime a crew dispatches to without
> saying so.

> **Note.** Earlier Orbit versions used `[agent.<role>]` tables. That schema was removed in [ORB-00058](../.orbit/) — config load now hard-errors if `[agent.*]` is present. Migrate to `[crews.<name>]` + `workflow.default_crew`.

---

## Provider identity and resolution

Every `provider` string Orbit reads — in `[crews.<name>]`, in an activity's inline `provider`, and in setup detection — is parsed through **one canonical surface** (`orbit_common::types::activity_job::Provider`, ORB-10091). Centralizing parsing means the crew resolver, the CLI executor, and reconciliation cannot disagree with each other or with Worker/Bridge about what a provider name means.

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
- **`openai_compat` has no CLI runtime.** Every crew dispatches through the CLI agent path, so selecting it fails structurally (see below) rather than falling back.

### Resolution precedence

Provider selection is **three composed steps**, not one. Describe them precisely — the inline `provider` on an activity is the *template baseline*, **not** an explicit override that outranks the crew.

**1 — Which crew is dispatched** (`resolve_crew_for_task`). The crew *name* is chosen by the Constellation provider-resolution precedence (contract §3), first non-empty tier wins:

1. **explicit** — `--crew` flag / run-input `crew`.
2. **task_config** — `task.crew` on the task artifact.
3. **workspace_default** — `[workflow].default_crew` in `config.toml`.
4. **environment_default** — the `CONSTELLATION_DEFAULT_PROVIDER` environment variable (a canonical provider id, which names the same-named single-family crew).
5. **system_default** — the canonical baseline (see below).

**2 — Which crew an activity uses.** A non-empty `crew` in the activity's rendered input selects that named crew. Without one, the activity uses the run's resolved crew from step 1. This is the only activity-authoring routing mechanism. Activity and job assets that declare `role` are rejected with guidance to pass `crew` in the activity input instead.

**3 — The activity crew's assignment overrides the inline baseline** (`resolve_from_config`). For each `(provider, model)` field independently: the selected crew value wins **when present**; otherwise the activity's inline `agent_loop` value stands. A crew assignment that omits a field (or whose `provider` string is unparseable) leaves the inline baseline in place — so a config typo never coerces dispatch onto a wrong runtime. This is also why **persisted provider identity is never re-defaulted** during reconciliation: a provider already frozen on a run record is reused verbatim, not reset to the enum default.

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

## Other sections (brief)

| Section | Purpose |
|---|---|
| `[execution.env]` | Env vars passed to agent subprocesses. Only the explicit `pass` list crosses the boundary; the process environment is never inherited wholesale. |
| `[execution.codex]` | Codex CLI sandbox mode. Valid: `read-only`, `workspace-write` (default), `danger-full-access`. Optional `approval_policy = "on-request"` enables escalation prompts. |
| `[tasks]` | `id_start = N` sets a floor for the local task-id allocator: on runtime build the counter is raised to at least `N` (never lowered), so machines can hold disjoint id ranges (e.g. one `0–9999`, another `10000+`) and avoid cross-machine collisions. Capped by `ORB_TASK_ID_MAX` (99999) — setting it near the ceiling shrinks the usable range. Prefer the one-shot `orbit workspace init --task-id-start N` for the initial seed; the config key keeps the floor sticky across machines that share a config. See [task-migration overview](design/task-migration/1_overview.md). |
| `[scoring]` | `enabled = true` records per-agent scoreboard counters under `.orbit/state/scoreboard/`. |
| `[pr]` | PR creation defaults (template, labels, draft mode) for `orbit run ship --mode pr`. |
| `[runtime]` | **JSONL log rotation/retention** (`~/.orbit/state/logs/orbit.jsonl`): `log_retention_days` (default `7`) deletes archives older than N days; `log_max_total_mb` (default `500`) caps total archive size, pruning oldest first; `log_max_file_mb` (default `100`) rolls the active file to a dated archive once it exceeds N MiB. Rotation runs opportunistically at process start. Invalid values (`0`, or `log_max_file_mb > log_max_total_mb`) are rejected at config load. |

---

## Validation and errors

Config is parsed at startup; invalid entries fail loud rather than silently falling back. Common failure modes:

- `[workflow].default_crew = '<x>' is not defined under [crews]` — name a crew that exists.
- `config schema changed in ORB-00058; remove [agent.<role>] tables` — migrate to crews.
- `execution.codex.sandbox has invalid value` — must be `read-only`, `workspace-write`, or `danger-full-access`.
- `tasks.id_start N exceeds maximum task id 99999` — the allocator start must fit the `ORB-00000` id space.
- `tasks.id_start N would lower the allocator below its current position M` — the counter only moves forward (raised only via `orbit workspace init --task-id-start`; the config key is a silent forward-only floor).

The runtime parser intentionally accepts sections owned by other readers of the shared file, such as `[docs]`. Consequently, retired keys with no runtime reader can remain syntactically accepted but have no effect. Existing configs containing `[duel]` and `[duel.models]` still load during the compatibility window and emit a warning naming both retired tables; remove them. The keys `execution.env.inherit`, `task.approval.delegate_approval`, and `task.approval.required_for_agent` are also inert and should be removed; environment inheritance is fixed off, while agent approval is enforced by the capability/policy surfaces rather than these old flags.

When in doubt, start with a minimal workspace file containing only genuine overrides. The annotated default ([`crates/orbit-core/assets/config/default-config.toml`](../crates/orbit-core/assets/config/default-config.toml)) is a reference for available settings, not a template that must be copied wholesale.
