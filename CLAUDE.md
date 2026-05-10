# CLAUDE.md

Project instructions for agents working on Orbit.

## Project Don'ts

- Don't commit until the Orbit task has been explicitly approved by the human.
- Don't invent task IDs — get them from `orbit.task.add`.
- Don't edit task files directly — use `orbit.task.update`.
- Don't add cross-crate dependencies without checking the architecture diagram below.
- When you hit friction, ambiguity, naming drift, or duplicated sources of truth: file a self-reported friction report via the `orbit-track-issues` skill instead of working around it.

## Project Do's

- Use subagents to support you through large tasks and keep your context window clean.

## Maintainer Conventions
- Order files for the next maintainer's first read: lead with the module's primary concept or entry point, then move into supporting detail.
- Data-heavy modules usually start with their core structs/enums; orchestration modules usually start with the main public function.

## Build / Lint

- Build: `make build`
- Fmt:   `make fmt`

All must pass before a task moves to `review`.

## Crate Architecture

```
orbit-common → orbit-policy, orbit-exec, orbit-knowledge, orbit-embed → orbit-tools → orbit-agent → orbit-engine → orbit-core → orbit-cli
            ↘ orbit-store ──────────────────────────────────────────────────────────────────────────↗            ↗
            ↘ orbit-embed-companion (depends on orbit-embed; not linked into the default `orbit` binary)
            ↘ orbit-mcp ─────────────────────────────────────────────────────────────────────────────────────────↗
```

- **orbit-common**: leaf — no internal deps. `types::` owns shared domain types, `OrbitError`, ID generation, and activity/job schemas; `utility::` owns generic helpers like fs, redaction, logging, and blob storage.
- **orbit-policy**: filesystem-scoping policy engine. Owns `FsProfile` resolution and `denyRead` / `denyModify` evaluation. Depends only on `orbit-common`.
- **orbit-exec**: process / sandbox / supervision primitives for shell-command execution under an `FsProfile`. Depends only on `orbit-common`.
- **orbit-embed**: semantic-embedding feature crate. Owns the `Embedder` trait, JSON-Lines RPC types, `SubprocessEmbedder`, `NoopEmbedder`, the workspace-local vector store (`vector::VectorStore` with its own `rusqlite::Connection`, WAL + busy_timeout pragmas, idempotent `embeddings` / `tasks_fts` schema, `EmbedWorker`, paragraph chunker, BLAKE3 dedup, cosine helper), and the install/uninstall/reindex/stats `commands::*` surface. Depends only on `orbit-common`; does not depend on `orbit-store` or fastembed-rs.
- **orbit-embed-companion**: separately installed embedding companion binary. Depends on `orbit-embed` and fastembed-rs; not linked into the default `orbit` CLI binary.
- **orbit-knowledge**: knowledge/graph parsing and storage helpers. Multi-language source parsing (Rust, Go, Java, JavaScript/TypeScript, Python). Depends on `orbit-common`; consumed by `orbit-tools`, which exposes graph tool and CLI-use-case facades upstream.
- **orbit-store**: layered store pattern (YAML + SQLite). Match existing modules when adding new ones. Depends only on `orbit-common`; the semantic vector schema is owned by `orbit-embed::vector` (not `orbit-store`).
- **orbit-tools**: tool registry plus built-in graph, fs, and policy-aware exec tools. Depends on `orbit-common`, `orbit-exec`, `orbit-knowledge`, `orbit-policy`.
- **orbit-mcp**: Model Context Protocol adapter using `rmcp`. Depends only on `orbit-common`; consumed by `orbit-cli` via `orbit mcp serve`.
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
| Skills          | MergeByKey         | Global defaults in `~/.orbit/skills`; workspace overrides by skill name |
| Command Audit   | GlobalOnly         | Single authoritative SQLite event trail          |
| Semantic Index  | WorkspaceOnly      | Task-derived embeddings stay with the workspace  |
| Run Traces      | WorkspaceOnly      | Per-repo activity/job JSONL and blob artifacts   |

## Orbit Workflow

For any Orbit lifecycle work (creating tasks, executing, reviewing, raising PRs), invoke the relevant `orbit-*` skill. The `orbit` skill is the entry point and router.

## Friction Reports

Friction reports are agent self-reports of Orbit tooling, workflow, skill, or seeded-instruction problems encountered during work.
Record them with `orbit.friction.add`; they are stored as append-only markdown records under `.orbit/frictions/`.
Do not use friction reports for user-requested work, backlog shaping, or generic bug tracking.
There is no accept/reject triage lifecycle for friction records.
No need to search for duplicates before reporting concrete friction.

## Activity / Job Model

The full design lives under [docs/design/activity-job/](./docs/design/activity-job/), with the current implementation in [docs/design/activity-job/2_design.md](./docs/design/activity-job/2_design.md).

### Activity YAML reference

Activity and job YAMLs declare `schemaVersion: 2`. Job steps reference activities by name via `target: activity:<name>` or inline the full spec via `spec:`. `agent_loop` activities declare `backend:`, `provider:`, and `wall_clock_timeout_seconds:`. A step inside a `loop:` body with a `session:` binding must resolve to `backend: http` (enforced at load time).

Policy is a filesystem-scoping surface only. Activities can declare `fsProfile: <name>` to select a named profile from the active policy; the policy layer contributes global `denyRead` / `denyModify` safety rails. If an activity omits `fsProfile:`, runtime resolves an implicit `unrestricted` profile (`read: [./**]`, `modify: [./**]`) before applying the global denies.

### Durable workflow state

Treat direct-agent stdout as an audit/diagnostic stream, not a workflow handoff channel. Jobs and activities should pass durable data through task artifacts, `orbit.state.*`, job-run state, or purpose-built tools such as `orbit.duel.plan.add` and `orbit.duel.plan.winner`; downstream steps should read those persisted records instead of parsing agent process output.

## Task Authoring Quality

Follow the `## Task Quality Standards` section in `orbit-create-task` skill: explicit observable definitions for summary fields (`purpose`, etc.), and testability-preserving implementation patterns.

**Commits & Tasks & doc authorship**:

- Use the agent commit identity (i.e. `codex` or `claude`) as author/committer when you make commits
- Include the task ID in the commit message when the commit is associated with an Orbit task (e.g. `[T20260320-001234]`).
- The task ID in the commit message is a local search key: it lets you or your agent find commits via `git log --grep '[T...]'` in your own workspace, and is not designed to be resolvable on another engineer's machine. When a task has a linked `external_ref`, include that tag alongside it (e.g. `[T20260506-11] [ENG-1234] ...`); cross-engineer reviewers resolve the external tag, not the Orbit one.
- Use your model name (e.g. `claude-opus-4-7`, `gpt-5.5`, `gemini-3.1-pro`) when authoring tasks or docs.
- When writing docs, cite relevant task IDs in the doc itself.
- When your change touches an owned feature's implementation, update that feature's design docs in the same PR: flip affected ADR statuses (`Proposed → Accepted` with task ID), bump `Last updated`, and add a new ADR for any non-obvious decision the change embodies. Stale docs are treated as a review blocker.

## Feature Ownership

Feature design docs live under `docs/design/<feature>/` and follow [`docs/design/CONVENTIONS.md`](docs/design/CONVENTIONS.md) (folder layout, required sections, ADR format, glossary shape).

| Feature | Folder | Lead |
|---------|--------|------|
| Knowledge graph | `knowledge-graph/` | `claude` |
| Policy & Sandboxing | `policy-sandbox/` | `claude` |
| Project Learnings | `project-learnings/` | `claude` |
| Task Lineage | `task-lineage/` | `claude` |
| Activity / Job | `activity-job/` | `codex` |
| Auditability | `auditability/` | `codex` |
| Groundhog | `groundhog/` | `codex` |
| User Interface | `user-interface/` | `gemini` |

## Scoreboards

Scoreboards live at `.orbit/state/scoreboard/`:

- `duel_plan.json` — planning-duel run results.
