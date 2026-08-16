# Configuration

What is tunable and where. The canonical, field-by-field reference is the
project's `docs/CONFIG.md` — read that for exhaustive semantics. This covers the
settings that change how work actually runs.

## Where config lives

| Path | Scope | Created by |
|---|---|---|
| `<workspace>/.orbit/config.toml` | Workspace-local | Hand-authored, optional |
| `~/.orbit/config.toml` | Global / user | `orbit init` |

Ordinary settings inherit per key: workspace overrides global, global fills
omissions, built-in defaults fill the rest. Tables layer down to individual
settings; scalars and arrays replace wholesale.

**Three security-sensitive settings deliberately do not inherit** once a
workspace file exists — `execution.codex.sandbox`,
`execution.codex.approval_policy`, and `execution.env.pass`. If the workspace
file omits one, its built-in default applies rather than the user's global
value. This keeps repository sandboxing deterministic instead of dependent on
whoever's machine is running.

```bash
orbit config show                  # effective merged view, with source provenance
orbit config show --scope global   # one physical file
orbit config get <key>
orbit config set <key> <value>
orbit config keys                  # every settable key
orbit config path
```

`orbit config show` annotates every setting as `workspace`, `global`,
`built-in`, or `environment`, with the file it came from. Reach for it before
assuming a value.

## Settable keys

| Key | Purpose |
|---|---|
| `workflow.base_branch` | Default base branch for ship workflows. Set this first — it decides where PRs land. |
| `workflow.default_crew` | Crew for any task that doesn't declare one. |
| `workflow.system_crew` | Crew for Orbit's own bounded activities (failure recovery, triage). |
| `workflow.auto_ship` | Opt-in for unattended ship dispatch via the scheduler. |
| `routines.role` | `"source"` marks the workspace as a routine source for `orbit sweep`. |
| `tasks.id_start` | Floor for this machine's task-id allocator; forward-only. → [multi-host.md](multi-host.md) |
| `execution.env.pass` | Environment variable names allow-listed into agent subprocesses. |
| `execution.codex.sandbox` | `read-only`, `workspace-write`, or `danger-full-access`. |
| `execution.codex.approval_policy` | `untrusted`, `on-request`, or `never`. |
| `runtime.log_retention_days` | Delete JSONL archives older than N days. |
| `runtime.log_max_total_mb` | Total archive budget; oldest pruned first. |
| `runtime.log_max_file_mb` | Roll the active log past N MiB. Must not exceed the total budget. |
| `scoring.enabled` | Record scoreboard metrics for task runs. |
| `pr.task_url_template` | URL template linking a task ID in PR descriptions. |

## Crews

A crew is one named provider-and-model assignment. A task's `crew` field selects
its executor; `workflow.default_crew` covers the rest.

```toml
[crews.reviewer]
provider = "claude"
model = "opus"
description = "Deep review passes"
tags = ["review"]
```

Crews layer by name *and* field, so a workspace can override just the model of a
globally defined crew. A `default_crew` or `system_crew` naming an undefined crew
fails config load — deliberately, since the alternative is silently dispatching
to the wrong model.

`orbit init` seeds crews for the agent CLIs it detects, including the `qa` crew
that `system_crew` defaults to. Available providers:

```bash
orbit executor list
```

Executors are how a provider is invoked. You choose crews; executors are
infrastructure.

## Environment passthrough

Agent subprocesses always start from a **cleared** environment plus the
allowlist — the Orbit process's environment is never inherited, and that is not
configurable. The seeded baseline covers `HOME`, `PATH`, `TMPDIR`, `USER`, and
the provider home directories. Credentials are opt-in: add `GITHUB_TOKEN` or a
provider API key explicitly if agents need them.

```toml
[execution.env]
pass = ["HOME", "PATH", "TMPDIR", "USER", "GITHUB_TOKEN"]
```

## Policies and filesystem profiles

A policy defines the filesystem profiles activities run under.

```bash
orbit policy list
orbit policy show <name>
orbit policy check <path>        # dry-run a path against the active profile rules
```

Shipped profiles: `reviewer` and `pure_compute` (read-only), `docs_writer`
(scoped writes), `implementer` (workspace writes), `unrestricted`.

The trap worth knowing: an activity that omits `fsProfile` falls back to
unrestricted workspace writes. A read-only activity must declare its profile
explicitly — silence is not a safe default here.

## Docs roots

```toml
[docs]
roots = ["docs/"]
```

Add more with `orbit docs add <path>` rather than hand-editing.
→ [docs-corpus.md](../docs-corpus.md)
