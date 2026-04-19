# CLAUDE.md

Project instructions for agents working on Orbit.

## Project Don'ts

- Don't commit until the Orbit task has been explicitly approved by the human.
- Don't invent task IDs — get them from `orbit.task.add`.
- Don't edit task files directly — use `orbit.task.update`.
- Don't add cross-crate dependencies without checking the architecture diagram below.
- When you hit friction, ambiguity, naming drift, or duplicated sources of truth: file a self-reported friction task via the `orbit-track-issues` skill instead of working around it.
- Reserve task type `friction` for agent self-reports only. Do not use `friction` for normal user-requested work, backlog shaping, or generic bug tracking.
- DO NOT WRITE UNIT TESTS or RUN TESTS

## Project Do's

- Use subagents to support you through large tasks and keep your context window clean.

## Build / Lint

- Build: `make build`
- Fmt:   `make fmt`

All must pass before a task moves to `review`.

## Crate Architecture

```
orbit-common → orbit-policy, orbit-exec, orbit-knowledge → orbit-tools → orbit-store, orbit-agent → orbit-engine → orbit-core → orbit-cli
                                                         ↘ orbit-mcp ↗
```

- **orbit-common**: leaf — no internal deps. `types::` owns shared domain types, `OrbitError`, ID generation, and v2 schemas; `utility::` owns generic helpers like fs, redaction, logging, and blob storage.
- **orbit-knowledge**: knowledge/graph parsing and storage helpers. Depends on `orbit-common`; consumed by `orbit-tools` and `orbit-cli`.
- **orbit-store**: layered store pattern (YAML + SQLite). Match existing modules when adding new ones.
- **orbit-agent**: HTTP `LoopTransport` primitives **and** the retained `AgentRuntime` + `providers/*_cli.rs` CLI runtimes. Per the 2026-04-18 amendment to `docs/design/activity-job-v2.md` (§10.1 retention table), the CLI provider files are **kept** through Phase 5 as the implementation of v2 `backend: cli`. Phase 5 deletes the v1 asset format and v1 dispatch path; it does NOT delete `providers/*_cli.rs` or the `AgentRuntime` trait.
- **orbit-engine**: activity/job execution, template rendering, retry logic. Also owns the v2 `backend: cli` subprocess runner (`v2::cli_runner`), which names `orbit-agent::{Agent, AgentConfig}` directly — orbit-core stays clean of orbit-agent types per the T20260418-2210 boundary.
- **orbit-mcp**: Model Context Protocol adapter over `orbit-tools::ToolRegistry`. Depends only on `orbit-common`, `orbit-tools`, and `rmcp`; consumed by `orbit-cli` via `orbit serve mcp`.
- **orbit-core**: runtime bootstrap, config layering, command dispatch, default asset seeding. Exposes `V2RuntimeHost` via `impl` for `OrbitRuntime`; the trait surface is primitive (deterministic dispatch, API-key sourcing, CLI command resolution) so no orbit-agent type leaks through it.
- **orbit-cli**: clap-based CLI entry point.

## Scoping Rules

| Artifact        | Strategy           | Rationale                                        |
|-----------------|--------------------|--------------------------------------------------|
| Tasks           | WorkspaceOnly      | Per-repo backlog, no cross-project leaking       |
| Activities/Jobs | MergeByKey         | Global defaults + workspace overrides            |
| Policies        | MergeByKey         | Workspace overrides profiles by name; global `denyRead` / `denyModify` rules accumulate |
| Job Runs        | WorkspaceOnly      | Execution artifacts are workspace-local          |
| Skills          | WorkspaceReplaces  | Workspace has full control over available skills |
| Audit           | GlobalOnly         | Single authoritative event trail                 |

## Orbit Workflow

For any Orbit lifecycle work (creating tasks, executing, reviewing, raising PRs), invoke the relevant `orbit-*` skill. The `orbit` skill is the entry point and router.

## v1 → v2 Activity / Job Model

The full design lives at [`docs/design/activity-job-v2.md`](docs/design/activity-job-v2.md). The v1 → v2 concept mapping (§10.4) in short:

Current coexistence note: v1 compatibility is still live for the remaining legacy jobs and activities, but those runtime surfaces are deprecated. Today the high-signal v2 references are the activities under `crates/orbit-core/assets/activities/v2_reference/` and the sample job `crates/orbit-core/assets/jobs/v2_samples/task_pipeline.yaml`. Prefer `orbit activity run-v2` / `orbit job run-v2` for `schemaVersion: 2` YAML while the rest of the migration remains in flight.

| v1 | v2 |
|---|---|
| `spec_type: agent_invoke` + HTTP/SDK provider | `type: agent_loop` + `backend: http` |
| `spec_type: agent_invoke` + CLI provider (claude/codex/gemini/ollama) | `type: agent_loop` + `backend: cli` |
| `spec_type: automation` + `action: run_command` | `type: shell` |
| `spec_type: automation` + other actions | `type: deterministic` |
| `orbit.state.*` as in-run handoff | Pipeline context via `{{ steps.*.output.* }}`; `orbit.state.*` is cross-Run persistence only (§5) |
| Harness tool allowlist | Runtime-enforced allowlist for `backend: http`; advisory-only for `backend: cli` with a `tool_allowlist.harness_delegated` audit event per invocation (§6) |
| Subprocess stdout/stderr | Structured Run/Step/Activity events with verbatim blobs; §7.6 `cli.invocation.*` events for CLI-backend subprocesses |
| `finalize_tasks` integration gate | Per-unit PR + `agent-main` CI (T20260418-0421) |
| Delegate sub-tool surface (rejected T20260418-0538) | `agent_loop` + named `session:` binding |
| `max_iterations` + `loop_exit: true` | `loop:` construct with `break_when:` (§4.2) |
| `parallel_dispatch_tasks` activity | `fan_out:` / `fan_in:` construct (§4.2) |

### Activity YAML reference

v2 activity and job YAMLs declare `schemaVersion: 2`. Job steps reference activities by name via `target: activity:<name>` (Phase 4 name resolution) or inline the full spec via `spec:` (Phase 3 style, still supported for tests). `agent_loop` activities declare `backend:`, `provider:`, `wall_clock_timeout_seconds:` (§3.1). A step inside a `loop:` body with a `session:` binding must resolve to `backend: http` — enforced at load time (§3.2 item 1).

Policy is now a filesystem-scoping surface only. Activities can declare
`fsProfile: <name>` to select a named profile from the active policy, and the
policy layer contributes the global `denyRead` / `denyModify` safety rails. If
an activity omits `fsProfile:`, runtime resolves an implicit `unrestricted`
profile (`read: [./**]`, `modify: [./**]`) before applying the global denies.

## Task Authoring Quality

Follow the `## Task Quality Standards` section in `orbit-create-task` skill: explicit observable definitions for summary fields (`purpose`, etc.), and testability-preserving implementation patterns.

**Commits**:

- Use the agent commit identity (e.g. `claude`) as author/committer when the agent made the change.
- Include the task ID in the commit message when the commit is associated with an Orbit task (e.g. `[T20260320-001234]`).

## Scoreboards

- `.orbit/scoreboard/pr.json` — PR merge rates, revision counts, review comment validity per agent/model.
- `.orbit/scoreboard/friction_bounty.json` — self-reported agent friction reports (issues-reported, issues-accepted, issues-rejected) per agent/model. Rejected reports count against the reporter; quality over quantity.
