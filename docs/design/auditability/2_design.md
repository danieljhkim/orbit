# Auditability — Design

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-04-28 (T20260428-17)

This document describes Orbit's shipped auditability implementation across command audit rows, activity/job envelopes, loop-level provider/tool traces, blob storage, redaction, identity attribution, metrics-adjacent invocation records, and the current limitations that still need design attention. See [1_overview.md](./1_overview.md) for the feature's purpose and [3_vision.md](./3_vision.md) for forward-looking questions.

---

## 1. Storage Roots and Audit Channels

Auditability is currently split across five channels:

1. **Command audit records.** SQLite rows in the configured audit database. These back the public `orbit audit` query/export surface.
2. **V2 activity/job envelope events.** JSONL files under `.orbit/state/audit/v2_loop/{run_id}.jsonl`.
3. **Loop-level provider/tool events.** JSONL files under `.orbit/state/audit/loop/{run_id}.jsonl`.
4. **Global tracing events.** JSONL lines under `~/.orbit/state/logs/orbit.jsonl`.
5. **Invocation metrics.** SQLite records keyed by job run, activity, task, agent, model, usage, and tool-call summaries.

The split is intentional. Command audit rows are compact and queryable across invocations. Activity/job envelopes preserve workflow structure. Loop audit preserves provider and tool-call detail. The global tracing feed gives operators a live, tail-able stream before workspace runtime context exists. Invocation metrics support cost and scoreboard questions without scraping audit JSONL.

The run-trace storage-placement decision is [T20260426-0519]: file-backed run traces moved under `.orbit/state/audit/`, while command audit rows remain in the configured SQLite database. README now describes that split as "command audit events remain globally scoped in SQLite; file-backed activity/job run traces are workspace-local under `.orbit/state/audit`."

---

## 2. Command Audit Rows

The command audit schema is `AuditEvent` in `crates/orbit-common/src/types/audit_event.rs`. A row includes:

- `execution_id`
- timestamp
- command and subcommand
- optional tool, target type, and target id
- role
- status: `success`, `failure`, or `denied`
- exit code and duration
- working directory
- optional argument JSON, truncated stdout/stderr, error message, host, pid, and session id

The table and indexes live in `crates/orbit-store/migrations/0001_init.sql`, and the store implementation is `crates/orbit-store/src/sqlite/audit_event_store.rs`. The store can list, show, prune, compute stats, and return durations for p95 calculation.

The CLI path is an RAII guard in `crates/orbit-cli/src/audit_middleware.rs`. `AuditGuard` defaults to failure, marks success or denial explicitly, and writes one row in `Drop`. This means early returns still write an audit record as long as stack unwinding reaches the guard.

For `orbit tool run`, command-audit role attribution is model-first after [T20260427-52]. Orbit resolves one identity source at a time: `--input` or `--input-file` fields first, then explicit `--agent` / `--model` flags, then `ORBIT_AGENT_NAME` / `ORBIT_AGENT_MODEL`. When a source provides `model`, Orbit uses that exact model as the role label and infers the agent family from known model names for downstream task provenance. The legacy `agent` field and `--agent` flag remain accepted for compatibility, but if an explicit agent contradicts an inferable model family, the tool execution rejects the input instead of recording inconsistent provenance. If no identity source is present, the row uses `agent` as the fallback role because the tool-dispatch command surface is agent-facing by default. Direct non-tool CLI commands continue to use `admin`.

`crates/orbit-cli/src/main.rs` wraps non-audit commands in that guard after runtime initialization. Direct `orbit audit ...` commands are deliberately outside the guard today, so querying the audit log does not itself emit another command audit row.

After [T20260428-4], **tool-invocation audit ownership moved into the runtime layer** ([ADR-016](./4_decisions.md#adr-016--tool-invocation-audit-is-owned-by-the-runtime-with-mcp-preflight-bracketing)). `OrbitRuntime::execute_tool_command_dispatch` now writes the audit row for every tool dispatch — CLI tool-run, MCP `tools/call`, and any future entry point — with identical identity-resolution rules and policy chain. The dispatch carries a `ToolEntryPoint` discriminator (`Cli` | `Mcp`) that surfaces in the audit `subcommand` field as `"run"` (CLI) or `"run-mcp"` (MCP), so the same SQLite table can attribute calls back to their origin without a schema migration. Setup failures inside dispatch (e.g. an inconsistent `agent`/`model` rejected by `normalize_agent_family_for_model`) are wrapped into the same audit-write block so they too produce a failure-status row. `duration_ms` is clamped to `>= 1` at the audit-write site.

To prevent the legacy CLI `AuditGuard` from double-emitting on the `orbit tool run` path, the runtime sets a per-thread `mark_tool_audit_recorded` signal after writing, and the guard's `Drop` calls `take_tool_audit_recorded()` and skips its own emission when the flag is set. Paths that bail before the runtime is reached — invalid JSON, missing input, `--dry-run` short-circuit — leave the flag clear and continue to produce their existing guard-side audit row, so the per-invocation row count remains exactly one in every case.

---

## 3. Tool-Driven and Runtime Audit Records

Command audit records do not only come from the top-level CLI guard. Some runtime paths write targeted rows directly:

- `crates/orbit-core/src/command/tool.rs` (`execute_tool_command_dispatch`) records every tool invocation — CLI and MCP — as `command: tool` with `subcommand` set to `"run"` for CLI dispatch and `"run-mcp"` for MCP dispatch ([ADR-016](./4_decisions.md#adr-016--tool-invocation-audit-is-owned-by-the-runtime-with-mcp-preflight-bracketing)).
- `crates/orbit-cli/src/command/mcp/mod.rs` (`audited_mcp_call`) brackets the MCP `tools/call` preflight and dispatch so a rejected unknown / unexposed tool name (`ensure_mcp_tool_exposed` returning `Err`) produces a failure-status row tagged with `subcommand: "run-mcp"` even though the runtime is never reached.
- `crates/orbit-core/src/runtime/orbit_tool_host/mod.rs` records task lock reservation checks, reservations, releases, and denials with `target_type: task_reservation`.
- `crates/orbit-core/src/runtime/v2_host.rs` records gate-starvation failures for task bundles with `command: gate.starvation`.

These records use the same SQLite schema so `orbit audit list` and export commands can see them. The current design implication is important: audit producers are allowed below the CLI layer, but they must still use `AuditEventInsertParams` and preserve the same status, target, actor, and redaction expectations.

The prescriptive coverage expectations live in [specs/coverage-matrix.md](./specs/coverage-matrix.md).

After [T20260427-0023], selected high-signal runtime paths also emit live tracing projections without changing their canonical stores. Filesystem policy denials still write `FsCallEventKind::Denied` audit events, proc-spawn allowlist denials still return `OrbitError::PolicyDenied`, and friction task creation still increments the friction bounty scoreboard. The added tracing targets are `orbit.policy.deny` for FS/proc denials and `orbit.friction.reported` for friction submissions.

---

## 4. Activity/Job Envelope Events

The v2 activity/job audit envelope type lives in `crates/orbit-common/src/types/activity_job/audit_envelope.rs`. The envelope fields are:

- `schemaVersion`
- `event_type`
- `event_id`
- timestamp
- `run_id`
- `agent_identity`
- optional `parent_event_id`
- optional `workspace_path`

The body is a tagged `V2AuditEventKind`. Current event families include:

- run lifecycle
- step lifecycle, retry, skip, denial, and join outcomes
- fan-out/fan-in worker state
- loop iteration lifecycle and non-convergence
- activity lifecycle
- filesystem request/result/denial
- tool denial
- CLI-backend allowlist delegation and subprocess lifecycle

`crates/orbit-engine/src/activity_job/audit_writer.rs` owns `V2AuditWriter`. It assigns event ids, stores parent stacks per thread, emits JSONL through `V2JsonlSink`, keeps an in-memory snapshot for smoke verification, and exposes the inner loop sink so provider/tool events can live beneath the envelope tree. CLI-launched v2 activity/job runs stamp envelope `agent_identity` as `system` because these records describe Orbit's workflow orchestration; concrete agent/provider identity is carried by activity configuration, CLI invocation events, and invocation metrics.

`crates/orbit-engine/src/activity_job/jsonl_sink.rs` writes one JSON object per line under `v2_loop/`, append-only for the life of a run and flushed per write.

`crates/orbit-core/src/runtime/run_audit.rs` owns the read-side accessor for this file-backed layer after [T20260426-0709]. It collects v2 envelope events, derives activity DAG `step.id` values from `parent_event_id` ancestry, and resolves CLI stdout/stderr blob references for `orbit run logs` without exposing audit storage paths to the CLI renderer.

---

## 5. Loop-Level Provider and Tool Events

The lower-level loop audit type is `LoopAuditEvent` in `crates/orbit-agent/src/loop_engine/audit/mod.rs`. Its events cover:

- session spawn and close
- HTTP request and response
- tool-call request and result
- iteration boundary
- policy denial

`JsonlFileSink` writes loop events to `{audit_root}/loop/{run_id}.jsonl`. The same sink writes payload blobs to `{audit_root}/blobs/` and references them by sha256. Orbit runtime callers pass `.orbit/state/audit` as `audit_root`, so loop traces share the same workspace-local audit root as v2 envelopes.

The loop engine emits hashes for request bodies, response bodies, tool inputs, and tool outputs rather than embedding those bodies in the event row. This keeps event lines queryable while still preserving replay material in blob storage.

---

## 6. Blob Storage and Redaction

`crates/orbit-common/src/utility/blob_store.rs` writes content-addressed blobs under `{root}/{hash_prefix}/{hash}`. The hash is computed after redaction, and an existing blob path is reused without rewriting.

`crates/orbit-common/src/utility/redaction.rs` centralizes two redaction mechanisms:

- sensitive live environment values, selected by environment variable name patterns such as `SECRET`, `TOKEN`, `PASSWORD`, `API_KEY`, `AUTH`, and related names
- regex-based HTTP and argv patterns for authorization headers, x-api-key fields, bearer tokens, JSON API keys, and bare `sk-...` tokens when argv scrubbing is requested

The CLI audit guard redacts error messages before writing them. The blob store redacts bytes before writing them. The pipeline runtime also redacts JSON outputs and errors before persisting selected pipeline data. After [T20260426-2349], the default tracing subscriber also redacts string field values, `Debug`-formatted field values, and unstructured `message` fields before writing stderr or global JSONL tracing output.

The smoke example `crates/orbit-agent/examples/redaction_smoke.rs` verifies that stored blob bytes omit the raw secret and contain a redaction marker.

---

## 7. Identity and Attribution

Orbit currently carries identity through several related fields:

- CLI runtime actor identity defaults direct CLI commands to `human`.
- `orbit tool run` paths prefer exact `model` inputs for provenance; known model names infer the task `agent` family, while legacy explicit `agent` inputs are compatibility hints that must agree with the model.
- `V2AuditEnvelope.agent_identity` records the workflow-envelope actor. CLI-launched v2 runs use `system`; concrete agent activity is recorded in provider-specific event bodies and invocation metrics.
- Task records carry `created_by`, `planned_by`, `implemented_by`, `agent`, and `model` fields.
- Invocation metrics record agent and model beside job run and activity ids.

Task attribution remains automatic by default. Non-empty plan writes stamp `planned_by` from the effective actor label, and transitions into `review` or `done` stamp `implemented_by` from the task/model identity path. After [T20260427-47], `orbit.task.update` and the direct `orbit task update` CLI also accept explicit `planned_by` and `implemented_by` values for corrective provenance edits; explicit values win over automatic stamping for the same update, and empty strings clear the fields.

The core design requirement is not that all of these fields collapse into one value. It is that a reviewer can follow the chain from task state, command rows, v2 run envelope, provider/tool trace, and metrics record back to a concrete human or model identity.

This area is still uneven. Direct command rows still store role strings such as `admin`; agent-facing tool rows store resolved actor labels; some task paths normalize model names into attribution labels. The design intent is clear, but a unified identity glossary and query join story remain open.

---

## 8. Query, Export, and Metrics Surfaces

`crates/orbit-cli/src/command/audit.rs` exposes command audit rows through:

- `orbit audit list`
- `orbit audit show <id>`
- `orbit audit stats`
- `orbit audit export --format json`
- `orbit audit export --format csv`
- `orbit audit prune`

The command audit query surface supports filters for time, tool, status, role, and limit. Exports include all command-audit columns, including currently sparse fields such as `stdout_truncated`, `stderr_truncated`, and `session_id`.

V2 JSONL traces are exposed through run-scoped inspection commands after [T20260426-0705]:

- `orbit run events [run_id]` prints chronological envelope events and supports step and event-type filters.
- `orbit run trace [run_id]` renders the `event_id` / `parent_event_id` tree.
- `orbit run logs [run_id]` extracts CLI stdout/stderr blobs from CLI invocation envelope events.

These are intentionally separate from `orbit audit`, which remains the compact SQLite command-audit query surface. `orbit run history` and `orbit run show` expose durable job-run state rather than the full envelope stream.

Invocation metrics are surfaced through metrics and scoreboard commands. They are useful for cost and usage analysis, but they do not replace the audit trail because they summarize rather than preserve transcript structure.

After [T20260428-11], `summary.json` uses command-audit rows as the source for all audited Orbit tool-run attempts and failed tool-run attempts. The query counts `command: tool` rows with `subcommand: "run"` or `"run-mcp"` and a concrete `tool_name`, grouped by normalized role/model. `tool_calls` remains the public all-call field; `failed_tool_calls` counts non-success tool-run rows (`failure` and `denied`). Token totals still come from the invocation/token scoreboard, and legacy token-scoreboard tool-call totals remain a fallback through a max overlay rather than an additive merge to avoid obvious double counting between model traces and command-audit rows.

After [T20260428-17], local Orbit task review feedback and GitHub PR review feedback are separate scoreboard inputs. Creating or replying to a local task review thread records `task-review-messages` in `.orbit/state/scoreboard/task_review.json`; successful GitHub sync of pending review-thread messages records `pr-review-comments` in `.orbit/state/scoreboard/pr.json`. GitHub sync accepts legacy `agent / model` labels plus model-only labels that exactly match the configured orchestrator/helper model for an inferred agent family; it skips `human`, `system`, and arbitrary bare labels. `summary.json` schema version 2 exposes those counters as `task_review.messages` and `pr.review_comments` respectively so dashboards can show both without mixing local review activity with PR review activity.

After [T20260427-43], the friction bounty scoreboard is refreshed from task history rather than trusted only as an increment log. `type: friction` tasks are counted as reported, `status: friction` exits to `backlog`, `in-progress`, or `done` count as accepted, and `status: friction` exits to `rejected` count as rejected. The task store migration that moves legacy untriaged friction reports from `proposed/` into `friction/` keeps the scoreboard derivation tied to lifecycle history instead of current status alone.

---

## 9. Global Process Tracing JSONL

`crates/orbit-common/src/utility/logging.rs` installs the default tracing subscriber. After [T20260426-2343], that subscriber is a `tracing_subscriber::Registry` with one shared `EnvFilter`, the existing stderr fmt layer, and an optional JSON Lines file layer.

The file layer opens `~/.orbit/state/logs/orbit.jsonl` in append mode and writes through `tracing_appender::non_blocking`. The associated `WorkerGuard` is retained for the process lifetime inside `logging.rs`, so routine event emission does not synchronously block on disk writes.

Each JSONL record contains timestamp, level, target, and a `fields` object containing the structured event fields. After [T20260426-2349], `logging.rs` routes JSONL events through the same `RedactingFields` formatter used by stderr, so string field values, `Debug`-formatted field values, and unstructured messages are scrubbed before serialization while typed numeric and boolean values keep their JSON types. The timestamp is assigned when the formatter writes the record, so a non-blocking writer under load may introduce a small lag from event emission. This is the durable landing zone for live `tracing` events such as the CLI subprocess stdout/stderr events added in [T20260426-2313], the `orbit.policy.deny` events added for FS/proc policy denials in [T20260427-0023], and the `orbit.friction.reported` events added for friction submissions in [T20260427-0023].

This channel is global rather than workspace-local because `orbit-cli` initializes logging before clap parsing and before `OrbitRuntime` can resolve workspace roots. It is not a replacement for command audit rows or run traces: readers should treat it as an operational log stream, not the canonical workflow envelope.

---

## 10. Concerns & Honest Limitations

1. **Tamper evidence is promised more strongly than it is implemented.** README describes append-only and tamper-evident retention, but the current SQLite rows and JSONL files do not yet have a hash chain, signature, or external transparency log.
2. **Audit is split across storage systems.** Command audit rows, v2 JSONL, loop JSONL, blobs, job-run state, and invocation metrics are related by ids but not yet joined by one operator command.
3. **`orbit audit` does not audit itself.** That avoids recursive noise, but it also means audit-log reads, exports, and prunes are not themselves recorded through the normal command guard.
4. **Some command-audit fields are placeholders.** `stdout_truncated`, `stderr_truncated`, and `session_id` exist in the schema but are often `None`.
5. **CLI backend tool enforcement remains weaker than HTTP.** Activity/job audit records the CLI backend allowlist as harness-delegated. That preserves accountability but does not enforce Orbit-level tool denial semantics in the CLI provider path.
6. **Redaction favors known secret shapes.** Environment-value and regex redaction cover common provider-key paths, but no redactor can prove arbitrary user secrets are absent from every payload.
7. **The global tracing feed is intentionally v1-simple.** It has no rotation and no cross-process line lock; readers should tolerate rare malformed lines if concurrent processes interleave large writes. Redaction covers known string secret shapes, but span attributes and binary blobs are separate concerns.
8. **Coverage is still expanding.** Some deterministic actions and direct runtime mutations write explicit audit rows; others rely on enclosing command/job context. The coverage matrix must become the review checklist for new mutation paths.

---

## Task References

- **[T20260419-0002]** — Add workspace provenance and v2 audit envelope events for activity/job execution.
- **[T20260426-0519]** — Move file-backed activity/job audit traces under workspace state.
- **[T20260426-0526]** — Persist v2 invocation traces for metrics beside audit.
- **[T20260426-0605]** — Add this auditability design folder and document the current audit architecture.
- **[T20260426-0705]** — Expose v2 run audit events through `orbit run events` and `orbit run trace`.
- **[T20260426-0709]** — Align run step selectors on activity `step.id` and move CLI invocation log reading behind orbit-core runtime accessors.
- **[T20260426-0742]** — Remove duplicate job-level run inspection aliases and keep run inspection under `orbit run`.
- **[T20260426-2313]** — Stream CLI subprocess stdout/stderr through structured tracing events while retaining the existing audit/blob path.
- **[T20260426-2343]** — Add the global process tracing JSONL feed at `~/.orbit/state/logs/orbit.jsonl`.
- **[T20260426-2349]** — Apply tracing-layer redaction before stderr and global JSONL output.
- **[T20260427-0023]** — Project policy denials and friction task submissions into the global tracing feed.
- **[T20260427-43]** — Add `status: friction`, creation-time type/status inference, migration, and history-derived friction bounty refresh.
- **[T20260427-47]** — Allow explicit task attribution correction for `planned_by` and `implemented_by` through task update paths.
- **[T20260428-4]** — Move tool-invocation audit ownership into the runtime, add the `ToolEntryPoint` discriminator (`run` / `run-mcp`), bracket MCP preflight + dispatch in `audited_mcp_call`, and add the per-thread CLI dedup signal so every tool invocation produces exactly one audit row from the right origin.
- **[T20260428-11]** — Derive `summary.json` all/failed tool-call counts from command-audit tool-run rows while keeping invocation/token scoreboard data as the token source.
- **[T20260428-17]** — Split local Orbit task-review scoring from PR review-comment scoring and surface both in compact scoreboards.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
