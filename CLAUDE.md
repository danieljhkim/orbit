# CLAUDE.md

Project instructions for agents working on Orbit.

## Project Don'ts

- Don't commit until the Orbit task has been explicitly approved by the human.
- Don't invent task IDs — get them from `orbit.task.add`.
- Don't edit task files directly — use `orbit.task.update`.
- Don't add cross-crate dependencies without checking the architecture diagram below.
- When you hit friction, ambiguity, naming drift, or duplicated sources of truth: file a self-reported friction task via the `orbit-track-issues` skill instead of working around it.
- Reserve task type `friction` for agent self-reports only. Do not use `friction` for normal user-requested work, backlog shaping, or generic bug tracking.

## Project Do's

- Use subagents to support you through large tasks and keep your context window clean.

## Build / Lint

- Build: `make build`
- Fmt:   `make fmt`

All must pass before a task moves to `review`.

## Crate Architecture

```
orbit-common → orbit-policy, orbit-exec, orbit-knowledge → orbit-tools → orbit-agent → orbit-engine → orbit-core → orbit-cli
            ↘ orbit-store ──────────────────────────────────────────────────↗            ↗
            ↘ orbit-mcp ─────────────────────────────────────────────────────────────────────────────────────────↗
```

- **orbit-common**: leaf — no internal deps. `types::` owns shared domain types, `OrbitError`, ID generation, and activity/job schemas; `utility::` owns generic helpers like fs, redaction, logging, and blob storage.
- **orbit-policy**: filesystem-scoping policy engine. Owns `FsProfile` resolution and `denyRead` / `denyModify` evaluation. Depends only on `orbit-common`.
- **orbit-exec**: process / sandbox / supervision primitives for shell-command execution under an `FsProfile`. Depends only on `orbit-common`.
- **orbit-knowledge**: knowledge/graph parsing and storage helpers. Multi-language source parsing (Rust, Go, Java, JavaScript/TypeScript, Python). Depends on `orbit-common`; consumed by `orbit-tools` and `orbit-cli`.
- **orbit-store**: layered store pattern (YAML + SQLite). Match existing modules when adding new ones. Depends only on `orbit-common`.
- **orbit-tools**: tool registry plus built-in graph, fs, and policy-aware exec tools. Depends on `orbit-common`, `orbit-exec`, `orbit-knowledge`, `orbit-policy`.
- **orbit-mcp**: Model Context Protocol adapter using `rmcp`. Depends only on `orbit-common`; consumed by `orbit-cli` via `orbit serve mcp`.
- **orbit-agent**: per-provider `AgentRuntime` implementations under `providers/<name>/<name>_runtime.rs` (claude, codex, gemini, openai_compat, anthropic, ollama, mock_agent). Implements `backend: cli`. Also hosts HTTP `LoopTransport` primitives.
- **orbit-engine**: activity/job execution, template rendering, retry logic. Owns the `backend: cli` subprocess runner (`activity_job::cli_runner`), which references `orbit-agent::{Agent, AgentConfig}` directly so orbit-core stays clean of orbit-agent types.
- **orbit-core**: runtime bootstrap, config layering, command dispatch, default asset seeding. Surfaces the `OrbitRuntime` API used by `orbit-cli`; does NOT depend on `orbit-agent`.
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

## Activity / Job Model

The full design lives under [docs/design/activity-job/](./docs/design/activity-job/), with the current implementation in [docs/design/activity-job/2_design.md](./docs/design/activity-job/2_design.md).

### Activity YAML reference

Activity and job YAMLs declare `schemaVersion: 2`. Job steps reference activities by name via `target: activity:<name>` or inline the full spec via `spec:`. `agent_loop` activities declare `backend:`, `provider:`, and `wall_clock_timeout_seconds:`. A step inside a `loop:` body with a `session:` binding must resolve to `backend: http` (enforced at load time).

Policy is a filesystem-scoping surface only. Activities can declare `fsProfile: <name>` to select a named profile from the active policy; the policy layer contributes global `denyRead` / `denyModify` safety rails. If an activity omits `fsProfile:`, runtime resolves an implicit `unrestricted` profile (`read: [./**]`, `modify: [./**]`) before applying the global denies.

## Task Authoring Quality

Follow the `## Task Quality Standards` section in `orbit-create-task` skill: explicit observable definitions for summary fields (`purpose`, etc.), and testability-preserving implementation patterns.

**Commits & Tasks & doc authorship**:

- Use the agent commit identity (i.e. `codex` or `claude`) as author/committer when you make commits
- Include the task ID in the commit message when the commit is associated with an Orbit task (e.g. `[T20260320-001234]`).
- Use your model name (e.g. `claude-opus-4-7`, `gpt-5.4`, `gemini-3.1-pro`) when authoring tasks or docs.
- When writing docs, cite relevant task IDs in the doc itself.
- Feature design docs live under `docs/design/<feature>/` and follow [`docs/design/CONVENTIONS.md`](docs/design/CONVENTIONS.md) (folder layout, required sections, ADR format, glossary shape). Feature leads: `claude` owns `knowledge-graph/`; `codex` owns `activity-job/` and `groundhog/`.
- When your change touches an owned feature's implementation, update that feature's design docs in the same PR: flip affected ADR statuses (`Proposed → Accepted` with task ID), bump `Last updated`, and add a new ADR for any non-obvious decision the change embodies. Stale docs are treated as a review blocker.

## Scoreboards

Scoreboards live at `.orbit/state/scoreboard/`:

- `friction_bounty.json` — self-reported agent friction reports (issues-reported, issues-accepted, issues-rejected) per agent/model. Rejected reports count against the reporter; quality over quantity.
- `duel_plan.json` — planning-duel run results.
