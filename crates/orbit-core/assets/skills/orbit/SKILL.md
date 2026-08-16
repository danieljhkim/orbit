---
name: orbit
description: Orbit — the task/audit/dispatch layer for AI coding agents. Use for anything touching a task ID, `.orbit/`, `orbit` CLI or `orbit.*` MCP tools, the docs corpus, friction records, jobs and `jrun-*` runs, routines, auto-tasks, the sweep clock, worktree GC, the dashboard, or setting Orbit up on a machine or repository. Covers both doing work through Orbit and configuring Orbit so that work runs unattended.
---

# Orbit

Orbit governs AI coding agents: every change is a task, every task carries an
audit trail, and work dispatches into sandboxed parallel runs. Durable
operations go through the registered tool surface — never direct lifecycle
subcommands, never a rebuild from source.

This file is the router. It carries only what every Orbit call needs; each
reference below is loaded on demand.

## Tool Invocation

Registered tools are reachable two ways, with identical JSON arguments:

| Surface | Form |
|---------|------|
| **MCP** — any client connected to `orbit mcp serve` | `orbit_task_add({...})` |
| **CLI** — inside an activity step, or with `orbit` on `PATH` | `orbit tool run orbit.task.add --input '{...}'` |

**Mapping rule:** `orbit.<group>.<action>` ↔ `orbit_<group>_<action>` — every dot
becomes an underscore, so `orbit.task.artifact.put` is `orbit_task_artifact_put`.
A gateway fronting the MCP mirrors this surface exactly; it may add an optional
`workspace` routing param and tools of its own, documented by that gateway.

**Always include `model`** — `codex`, `claude`, `gemini`, or `grok` — so Orbit
attributes the call. Full model strings auto-normalize; the family is the
persisted identity. This holds for every `orbit.*` call in every reference here.

`orbit tool list` (CLI) or `tools/list` (MCP) is the authoritative surface; never
guess a tool name. Two gaps worth knowing up front:

- **CLI-only:** `orbit semantic`, `orbit config`, `orbit doctor`, `orbit audit`,
  `orbit gc`, and the routine/sweep/job/auto-task commands. They need direct
  process access.
- **No query surface at all:** code structure (callers, refs, symbols,
  implementors). Inspect files with the provider-native file-read tool, or `rg`
  with shell access. Task-to-commit lookup is `git log --grep '[<task-id>]'`,
  not a graph query.

Some CLI flags have no MCP counterpart because they are already the MCP default:
`orbit tool run orbit.task.show --full` is plain `orbit_task_show({"id": "<id>"})`.
Pass `field` or `fields` to project instead. If an activity already injected
`task` into the execution envelope, read that snapshot instead of calling
`orbit.task.show` again.

## Lifecycle

```text
proposed → backlog → in-progress → review → done
         ↘ rejected

someday → in-progress
blocked → in-progress

review      → rejected
rejected    → backlog | in-progress  (reconsider)
```

Use `blocked` when execution cannot safely continue. Provenance follows the
surface: `orbit tool run ...` is agent-driven, bare `orbit task ...` is
human-driven.

## Common Mistakes — DO NOT

| Mistake | Why it fails | Correct form |
|---------|-------------|--------------|
| `cargo run -- tool run ...` | Rebuilds from source instead of using the installed binary | `orbit tool run ...` |
| `orbit task show <id>` | Direct CLI subcommands skip agent provenance tracking | `orbit tool run orbit.task.show --input '{"id":"<id>"}'` |
| Editing files under `.orbit/` | The store is authoritative; a file edit changes nothing a read returns | The matching `orbit.*` tool |
| Inventing a task ID | IDs are allocated by the store | `orbit.task.add` returns it |

## References

**Working through Orbit** — doing the work itself:

| Reference | Read it for |
|---|---|
| [concepts.md](references/concepts.md) | What each Orbit noun means and how they nest. Read this first if the vocabulary is new. |
| [task-authoring.md](references/task-authoring.md) | Writing a task someone else can execute without guessing. |
| [task-execution.md](references/task-execution.md) | Carrying a task from pickup to verified implementation and handoff. |
| [task-review.md](references/task-review.md) | Reviewing someone else's work against its acceptance criteria. |
| [search.md](references/search.md) | Finding prior tasks, docs, and frictions by topic, path, or related task. |
| [docs-corpus.md](references/docs-corpus.md) | Authoring and registering the markdown corpus agents retrieve from. |
| [friction.md](references/friction.md) | Recording what made the work harder than it should have been. |
| [orchestration.md](references/orchestration.md) | Driving a backlog through: `orbit run ship`, `run auto`, epics, and keeping parallel runs from colliding. |
| [workflows.md](references/workflows.md) | Jobs, activities, and the `orbit run` surface. |
| [run-debugging.md](references/run-debugging.md) | A `jrun-*` run that failed, stuck, or was cancelled. |
| [common-failures.md](references/common-failures.md) | Matching a known failure signature to its remedy, once the failing step is identified. |
| [operational-logs.md](references/operational-logs.md) | Host-level incidents: sweep service failures, tracing warnings, missing log output. |

**Setting Orbit up** — configuring a machine or repository so work runs well.
Most of Orbit's value is behind these, and a freshly initialized workspace has
all of it switched off:

| Reference | Read it for |
|---|---|
| [setup/first-run.md](references/setup/first-run.md) | Zero to a working workspace: install, host identity and task prefix, `workspace init`, MCP client registration, verification. |
| [setup/configuration.md](references/setup/configuration.md) | `orbit config` keys, crews and executors, policies and filesystem profiles, environment passthrough. |
| [setup/automation.md](references/setup/automation.md) | The scheduler: OS clock → `orbit sweep` → routines → jobs. The five seeded routines and the order to enable them. |
| [setup/auto-tasks.md](references/setup/auto-tasks.md) | Recurring work as data — definitions that mint tasks on a schedule, instead of new code. |
| [setup/maintenance.md](references/setup/maintenance.md) | Worktree GC, `orbit doctor` repairs, log retention, audit pruning, upgrades. Skipping the first is what breaks busy workspaces. |
| [setup/multi-host.md](references/setup/multi-host.md) | More than one machine: task-ID namespaces, routine host pins, and what syncs versus what stays local. |
| [setup/remote-access.md](references/setup/remote-access.md) | Reaching another machine's Orbit: the dashboard, `web connect`, and MCP over SSH or a socket. |

## Start here

- New to Orbit, or `.orbit/` is absent → [concepts.md](references/concepts.md), then [setup/first-run.md](references/setup/first-run.md).
- Given a task ID → [task-execution.md](references/task-execution.md).
- Asked to create work → [task-authoring.md](references/task-authoring.md).
- "Why didn't this fire / run / clean up?" → [setup/automation.md](references/setup/automation.md), then [setup/maintenance.md](references/setup/maintenance.md).
- A run id in hand → [run-debugging.md](references/run-debugging.md).
