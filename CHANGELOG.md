# Changelog

## Unreleased

### Breaking Changes

- **Activity/job v1 retired**: `schemaVersion: 1` activity/job assets are rejected at load time, the old v1 reconcile/runtime/store paths were removed, and `schemaVersion: 2` is now the canonical activity/job surface ([T20260419-2156], [T20260420-0036])
- **Workflow CLI surface reorganized**: workflows now use `orbit run ship <TASK_ID>...`, `orbit run ship --mode local <TASK_ID>...`, `orbit run ship-auto`, `orbit run duel-plan <TASK_ID>`, and `orbit run job <JOB_ID>`; workflow-specific `run ship list/show` and `run duel list/show` moved to the generic `orbit job history` / `orbit job run-state` inspection surface ([T20260417-0248], [T20260419-0355], [T20260425-2010])
- **`orbit task history` moved to `orbit graph history`**: the task-ID attribution query is a graph operation (selectors, sidecar, manifest are graph-owned) and now lives next to `orbit graph search`/`show`. The `orbit task history rebuild` subsubcommand is gone — run `orbit graph build` directly. New `--task-id-pattern <regex>` flag on both `orbit graph build` and `orbit graph history` makes task-ID extraction configurable for non-Orbit codebases (Jira, Linear, GitHub-issue conventions); workspace config `knowledge.task_id_pattern` is the steady-state setting, with resolution order CLI flag > config > Orbit default. Pattern is recorded in `manifest.json`; mismatch triggers a stderr warning. New `orbit.graph.history` agent tool exposes the same query surface to MCP clients ([T20260426-0507])

### Features

- **Activity/job v2 runtime**: added schema v2 activities and jobs with typed DAG blocks (`parallel`, `fan_out`, `loop`, `retry`, `when`), activity name resolution, `backend: auto` normalization, `backend: cli` dispatch, HTTP agent loops, session-bound loop steps, and a v2 audit envelope with workspace provenance ([T20260418-2018], [T20260418-2019], [T20260418-2143], [T20260418-2210], [T20260419-0002], [T20260419-0104])
- **Seeded task pipelines**: added load-bearing seeded workflows for PR, local, gate, auto-dispatch, and epic shipment, including task reservations, backlog bundling, admission-controlled dispatch, and session-backed epic orchestration ([T20260419-0622-3], [T20260419-0623], [T20260419-0623-2], [T20260419-2347])
- **Knowledge graph**: added the Rust `orbit-knowledge` graph, `orbit graph build/update/search/show`, graph MCP tools, compact overviews, callers/implementors/dependency navigation, graph edit buffering, shared locks, auto-refresh, branch-scoped refs, task-ID attribution, `orbit task history`, and markdown/config/table extraction ([T20260411-0424], [T20260412-0645-2], [T20260412-0645-3], [T20260421-0358], [T20260421-0528], [T20260422-1540])
- **MCP integrations**: added the `orbit-mcp` crate, `orbit mcp serve`, safe default graph/task tool exposure, external MCP/plugin tooling, and `orbit mcp init/remove` setup for Claude, Codex, and Gemini clients ([T20260418-0336], [T20260419-0236], [T20260422-1713], [T20260426-0354])
- **Dashboard and observability**: added `orbit web serve`, task/job/audit/scoreboard/dashboard APIs, diagnostics and recent-runs views, task actions from the dashboard, copyable task IDs, connection health, skeleton/loading states, markdown rendering, and live-data animations ([T20260417-0346], [T20260417-0412], [T20260417-0427], [T20260417-0437], [T20260417-0528], [T20260418-2004], [T20260426-0354])
- **Task planning and search**: added structured task plans, dependency support, epic task type support, selector-first task context, agent task search, and richer task field projection for agent/tool callers ([T20260419-2300], [T20260420-0509-2], [T20260420-0521], [T20260421-0445], [T20260422-1756])
- **Groundhog execution model**: added Groundhog chronicle serialization, workspace snapshots, verb tools, checkpoint verification, and a dedicated Groundhog v1 activity runner ([T20260420-0509], [T20260420-0509-3], [T20260420-0509-4], [T20260420-0510], [T20260420-0510-2])
- **Provider and evaluation support**: added Gemini support, configurable agent/model selection, provider invocation traces, HTTP LoopTransport implementations for Anthropic/OpenAI-compatible/Gemini providers, planning duels, scoreboard attribution improvements, and versioned knowledge-graph benchmark harnesses ([T20260411-1937-2], [T20260412-0457-2], [T20260412-1939], [T20260412-2129], [T20260418-0645], [T20260418-0759], [T20260422-1609])

### Fixes

- **Job run observability**: `orbit run ship --json`, `orbit job history`, `orbit job run-state`, and direct `orbit job run` now retain actionable failure details and durable run-state/job-history records, including synthetic job-level steps for early v2 pipeline failures ([T20260423-0445], [T20260423-2004-4], [T20260425-2010])
- **Branch-scoped knowledge graph refs**: graph builds now write `.orbit/knowledge/graph/refs/heads/<branch>.json` files that point at immutable per-build indexes, reads default to the current git branch with default-branch fallback, and legacy `.orbit/knowledge/graph/refs/current.json` stores auto-migrate on first open/write ([T20260421-0358])
- **Knowledge graph hardening**: graph reads and refreshes recover from corrupted stores, avoid stale worktree data, gate refresh/search hot paths, prune missing context files from locks, and hydrate task IDs idempotently during attribution ([T20260416-0719], [T20260417-0307], [T20260420-0540], [T20260421-0652])
- **Dispatch and locking correctness**: task locks now detect directory/file overlaps, backlog selection filters locked groups, failed task-scoped runs move tasks to blocked with job/run/error context, and drained local batches no longer fail spuriously ([T20260412-0443], [T20260417-0301], [T20260419-2109], [T20260420-0014])
- **Workflow compatibility**: merged object-valued job defaults with caller input, aligned the Quick Start approval flow with the current task lifecycle, and consolidated retired workflow inspection under generic job history commands ([T20260423-0445], [T20260423-0447], [T20260423-2004-2], [T20260425-2010])
- **Release and developer tooling**: restored release CI targets, repaired advertised developer targets, kept custom roots isolated, and fixed crashes/empty listings after seeded activity/job initialization ([T20260419-2347], [T20260423-2004], [T20260423-2004-3], [T20260423-2004-5])
- **Security and concurrency hardening**: added localhost origin checks for web write endpoints, serialized diagnostics JSONL appends, hardened task-store concurrency, tightened filesystem/tool-runtime path boundaries, and strengthened agent protocol handling ([T20260417-0557], [T20260417-0558], [T20260418-1928])

### Chores

- **Crate architecture**: extracted `orbit-common`, `orbit-knowledge`, and `orbit-mcp`, merged the older `orbit-types` surface into `orbit-common`, decomposed execution/runtime modules, and kept crate dependency direction aligned with the documented architecture ([T20260411-0008], [T20260419-2014])
- **Documentation and positioning**: added Orbit positioning docs, design-doc conventions, activity-job/knowledge-graph/Groundhog design docs, benchmark reports, and README updates for the current workflow and MCP surfaces

## 0.2.0

### Features

- **Parallel batch execution**: dispatch and execute multiple tasks in parallel with file-level conflict detection and shared worktrees
- **Auto-cleanup on merge**: ship workflow now deletes the remote branch after a successful PR merge

### Fixes

- **`--parallelism` flag**: serialized as JSON integer instead of string, fixing schema validation failure on `orbit run ship --parallelism N`
- **Stale default artifacts**: `orbit workspace init` now always refreshes default skills, activities, and jobs to their latest embedded versions (custom artifacts are preserved)
- **Clippy warning**: resolved unused-mut warning and removed clippy from CI

### Chores

- Default branch renamed from `agent-main` to `main`
- Removed `orbit` label from PR creation
- Agent configuration updates

## 0.1.0

Initial release of Orbit.

### Core

- **Task lifecycle**: propose, approve, implement, review, and archive tasks with full history tracking
- **Activity system**: reusable operations with defined input/output schemas and three spec types (agent_invoke, cli_command, automation)
- **Job engine**: composable multi-step pipelines with conditional execution, retry logic, nested jobs, and parallel dispatch
- **Workflow aliases**: `orbit run ship`, `orbit run ship-local`, `orbit run review` as ergonomic entry points over raw job invocation
- **Multi-agent orchestration**: parallel task workers with file-level locking in shared worktrees
- **Multi-model strategy**: configurable agent/model per job step (e.g., Opus for planning, Codex for implementation)

### CLI

- Grouped command surface: run workflows, manage work, configure and inspect
- JSON and table output modes across all commands
- Audit event logging for every CLI invocation

### Infrastructure

- Layered Rust crate architecture (types, policy, exec, tools, store, agent, engine, core, cli)
- Two-root workspace model: global (`~/.orbit/`) and workspace-local (`.orbit/`)
- File-based (YAML) and SQLite persistence
- RBAC policy evaluation engine
- Process sandboxing and timeout handling
- Skill system for agent prompt composition
