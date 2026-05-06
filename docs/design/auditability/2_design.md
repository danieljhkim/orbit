# Auditability — Design

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-06 (T20260506-2)

This document describes Orbit's shipped auditability implementation across command audit rows, activity/job envelopes, loop-level provider/tool traces, blob storage, redaction, identity attribution, metrics-adjacent invocation records, and known limitations. See [1_overview.md](./1_overview.md) for the feature purpose and [3_vision.md](./3_vision.md) for future questions.

---

## 1. Storage Roots and Audit Channels

Auditability is split across five channels:

1. **Command audit records.** SQLite rows in the configured audit database; queried through `orbit audit`.
2. **V2 activity/job envelope events.** JSONL under `.orbit/state/audit/v2_loop/{run_id}.jsonl`.
3. **Loop-level provider/tool events.** JSONL under `.orbit/state/audit/loop/{run_id}.jsonl`, created lazily when a run emits loop events.
4. **Global tracing events.** Redacted JSONL under `~/.orbit/state/logs/orbit.jsonl`.
5. **Invocation metrics.** SQLite records keyed by job run, activity, task, agent, model, usage, and tool-call summaries.

The split is deliberate: command rows stay compact and queryable; envelopes preserve workflow structure; loop audit preserves provider/tool detail; tracing gives operators a live feed before workspace context exists; metrics answer cost and scoreboard questions without scraping transcripts. [T20260426-0519] moved file-backed run traces under `.orbit/state/audit/` while command audit rows remained in SQLite.

---

## 2. Command Audit Rows

`AuditEvent` lives in `crates/orbit-common/src/types/audit_event.rs`. Rows include execution id, timestamp, command/subcommand, optional tool and target metadata, role, status, exit code, duration, working directory, optional argument/error/stdout/stderr fields, host, pid, and session id. The table and indexes live in `crates/orbit-store/migrations/0001_init.sql`; `crates/orbit-store/src/sqlite/audit_event_store.rs` lists, shows, prunes, exports, computes stats, and returns durations for p95 calculation.

After [T20260505-6], command-audit producers use the shared `audit_execution_id` helper instead of timestamp-only ids. The id keeps a stable producer prefix and appends wall-clock nanoseconds, process id, and a per-process atomic sequence so same-workspace parallel `orbit tool run ...` calls do not collide on clocks with coarse effective resolution. The SQLite unique index on `execution_id` remains the enforcement boundary.

The CLI RAII guard in `crates/orbit-cli/src/audit_middleware.rs` defaults to failure, marks success or denial explicitly, and writes one row in `Drop`, so early returns still audit when stack unwinding reaches the guard. Direct `orbit audit ...` commands are outside the guard today to avoid recursive audit noise.

For `orbit tool run`, [T20260427-52] made provenance model-first. Orbit resolves identity from input JSON or input file, then explicit `--agent` / `--model`, then `ORBIT_AGENT_NAME` / `ORBIT_AGENT_MODEL`. An exact model becomes the row role and can infer an agent family for task provenance; legacy `agent` inputs remain accepted only when consistent. Missing identity falls back to `agent` for tool dispatch, while direct non-tool CLI commands use `admin`.

After [T20260428-4], tool-invocation audit is written in `OrbitRuntime::execute_tool_command_dispatch` for CLI, MCP, and future entry points. A `ToolEntryPoint` discriminator surfaces as `subcommand: "run"` or `"run-mcp"`, setup failures inside dispatch are audited, and `duration_ms` is clamped to at least `1`. The legacy CLI guard skips its own emission when the runtime sets a per-thread `mark_tool_audit_recorded` signal; pre-runtime CLI failures such as invalid JSON still produce the existing guard-side row.

---

## 3. Tool-Driven and Runtime Audit Records

Some runtime paths write targeted command-audit rows directly:

- `crates/orbit-core/src/command/tool.rs` records CLI and MCP tool invocations as `command: tool` with `subcommand: "run"` or `"run-mcp"`.
- `crates/orbit-cli/src/command/mcp/mod.rs` records MCP preflight failures for unknown or unexposed tools before runtime dispatch.
- `crates/orbit-core/src/runtime/orbit_tool_host/mod.rs` records task lock reservation checks, reservations, releases, and denials.
- `crates/orbit-core/src/runtime/v2_host.rs` records gate-starvation failures for task bundles.

These producers share the SQLite schema and must preserve the same status, target, actor, and redaction expectations as CLI rows. Prescriptive coverage expectations live in [specs/coverage-matrix.md](./specs/coverage-matrix.md).

After [T20260427-0023], selected canonical stores also project live tracing events: filesystem policy denials still write FS audit events, proc-spawn allowlist denials still return `OrbitError::PolicyDenied`, friction task creation still updates the scoreboard, and each path also emits a redacted `orbit.policy.deny` or `orbit.friction.reported` event.

---

## 4. Activity/Job Envelope Events

`V2AuditEnvelope` lives in `crates/orbit-common/src/types/activity_job/audit_envelope.rs`. Each envelope carries `schemaVersion`, `event_type`, `event_id`, timestamp, `run_id`, `agent_identity`, optional `parent_event_id`, optional `workspace_path`, and a tagged `V2AuditEventKind`. Event families cover run, step, retry, skip, denial, join, fan-out/fan-in, loop, activity, filesystem, tool denial, CLI-backend delegation, and subprocess lifecycle.

`V2AuditWriter` in `crates/orbit-engine/src/activity_job/audit_writer.rs` assigns event ids, maintains per-thread parent stacks, emits through `V2JsonlSink`, keeps a smoke-verification snapshot, and exposes the inner loop sink for provider/tool events. CLI-launched v2 runs stamp envelope `agent_identity` as `system`; concrete agent identity lives in activity configuration, CLI invocation events, and invocation metrics.

`crates/orbit-engine/src/activity_job/jsonl_sink.rs` appends one JSON object per line under `v2_loop/` and flushes per write. `crates/orbit-core/src/runtime/run_audit.rs` is the read-side accessor after [T20260426-0709], deriving activity DAG `step.id` values from `parent_event_id` ancestry and resolving CLI stdout/stderr blob references for `orbit run logs`.

---

## 5. Loop-Level Provider and Tool Events

`LoopAuditEvent` in `crates/orbit-agent/src/loop_engine/audit/mod.rs` covers session spawn/close, HTTP request/response, tool-call request/result, iteration boundary, and policy denial. `JsonlFileSink` creates `{audit_root}/loop/{run_id}.jsonl` lazily on the first loop event and writes payload blobs to `{audit_root}/blobs/`; runtime callers pass `.orbit/state/audit` as `audit_root`. [T20260506-2] removed zero-byte loop JSONL placeholders for runs that only emit v2 envelope events or CLI-backend blobs.

Loop events reference hashes for request bodies, response bodies, tool inputs, and tool outputs instead of embedding the bodies inline. This keeps event lines queryable while preserving replay material in redacted blob storage.

---

## 6. Blob Storage and Redaction

`crates/orbit-common/src/utility/blob_store.rs` writes content-addressed blobs under `{root}/{hash_prefix}/{hash}`. The hash is computed after redaction, and existing blob paths are reused.

`crates/orbit-common/src/utility/redaction.rs` centralizes sensitive live environment value scrubbing plus regex-based HTTP/argv patterns for authorization headers, API keys, bearer tokens, JSON API-key fields, and bare `sk-...` tokens when argv scrubbing is requested. CLI audit errors, blob bytes, selected pipeline outputs/errors, and the default tracing subscriber all redact before persistence or terminal/JSONL output. The smoke example `crates/orbit-agent/examples/redaction_smoke.rs` verifies stored blob bytes omit the raw secret and contain a marker.

---

## 7. Identity and Attribution

Orbit currently carries identity through related fields rather than one universal key:

- Direct CLI commands default to a human/admin-facing role; agent-facing tool rows prefer exact `model` labels and infer compatible agent families.
- `V2AuditEnvelope.agent_identity` records the workflow-envelope actor. CLI-launched v2 runs use `system`; concrete provider activity appears in event bodies and metrics.
- Task records carry `created_by`, `planned_by`, `implemented_by`, `agent`, and `model`.
- Invocation metrics record agent and model beside job run and activity ids.

Task attribution remains automatic by default: non-empty plan writes stamp `planned_by`, and transitions into `review` or `done` stamp `implemented_by`. After [T20260427-47], `orbit.task.update` and direct `orbit task update` can explicitly set or clear those fields; explicit values win within the same update.

The requirement is not to collapse every field into one value. It is that a reviewer can follow task state, command rows, run envelopes, provider/tool traces, and metrics back to a concrete human or model. A unified identity glossary and query join story remain open.

---

## 8. Query, Export, and Metrics Surfaces

`crates/orbit-cli/src/command/audit.rs` exposes command rows through `orbit audit list`, `show`, `stats`, `export --format json`, `export --format csv`, and `prune`, with filters for time, tool, status, role, and limit. Exports include all command-audit columns, including sparse `stdout_truncated`, `stderr_truncated`, and `session_id`.

V2 traces are exposed separately: `orbit run events` prints chronological envelopes, `orbit run trace` renders the parent tree, and `orbit run logs` extracts CLI stdout/stderr blobs. `orbit run history` and `orbit run show` expose job-run state rather than the full envelope stream. Metrics and scoreboard commands read invocation records; they summarize cost and usage, not transcript structure.

After [T20260428-11], compact `summary.json` counts all audited tool-run attempts and failed attempts from command-audit rows where `command: tool`, `subcommand` is `"run"` or `"run-mcp"`, and `tool_name` is present. Token totals still come from invocation/token scoreboards, with legacy tool-call totals used only as a max overlay to avoid obvious double counting.

After [T20260428-17] and [T20260430-4], local task review and GitHub PR review are separate scoreboard inputs. Local review-thread creations record `task-review-threads` in `task_review.json`; successful GitHub sync records `pr-review-comments` in `pr.json`. `summary.json` schema version 2 exposes these as `task_review.threads` and `pr.review_comments`, and scoring accepts only exact configured model identities or built-in defaults, skipping `human`, `system`, and arbitrary bare labels.

After [T20260427-43], the friction bounty scoreboard is refreshed from task history: `type: friction` counts reports, exits from `status: friction` to `backlog`, `in-progress`, or `done` count acceptances, and exits to `rejected` count rejections.

---

## 9. Global Process Tracing JSONL

`crates/orbit-common/src/utility/logging.rs` installs a default subscriber with one `EnvFilter`, stderr formatting, and an optional non-blocking JSONL file layer at `~/.orbit/state/logs/orbit.jsonl` after [T20260426-2343]. The retained `WorkerGuard` lets routine event emission avoid synchronous disk writes.

Each record contains timestamp, level, target, and structured fields. After [T20260426-2349], both stderr and JSONL use `RedactingFields`, which scrubs string values, `Debug`-formatted values, and unstructured messages while preserving numeric and boolean JSON types. This global feed is the live landing zone for subprocess output [T20260426-2313], policy-denial and friction projections [T20260427-0023], and other `tracing` events emitted before workspace runtime context exists. It is operational telemetry, not the canonical workflow envelope.

---

## 10. Concerns & Honest Limitations

1. **Tamper evidence is promised more strongly than implemented.** SQLite rows and JSONL files do not yet have hash chains, signatures, or external transparency logs.
2. **Audit is split across stores.** Command rows, v2 JSONL, loop JSONL, blobs, job-run state, and invocation metrics share ids but lack one joined operator command.
3. **`orbit audit` does not audit itself.** That avoids recursion but leaves audit reads, exports, and prunes outside the normal guard.
4. **Some command-audit fields are sparse.** `stdout_truncated`, `stderr_truncated`, and `session_id` often remain `None`.
5. **CLI backend tool enforcement is weaker than HTTP.** Activity/job audit records the CLI backend allowlist as harness-delegated rather than enforcing Orbit-level tool denial semantics inside the provider path.
6. **Redaction covers known secret shapes.** Environment-value and regex redaction reduce risk but cannot prove arbitrary user secrets are absent from every payload.
7. **The global tracing feed is v1-simple.** It has no rotation and no cross-process line lock; readers should tolerate rare malformed lines if concurrent processes interleave large writes.
8. **Coverage is still expanding.** Some deterministic mutations write explicit audit rows; others rely on enclosing command/job context. The coverage matrix should become the review checklist for new mutation paths.

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
- **[T20260428-4]** — Move tool-invocation audit ownership into the runtime, add the `ToolEntryPoint` discriminator, bracket MCP preflight + dispatch, and deduplicate CLI guard rows.
- **[T20260428-11]** — Derive `summary.json` all/failed tool-call counts from command-audit tool-run rows while keeping invocation/token scoreboard data as the token source.
- **[T20260428-17]** — Split local Orbit task-review scoring from PR review-comment scoring and surface both in compact scoreboards.
- **[T20260430-4]** — Change local task-review scoring to count review-thread creations rather than replies, rename the compact field to `task_review.threads`, and keep legacy metric reads mapped forward.
- **[T20260430-20]** — Shorten the auditability docs while preserving required guarantees.
- **[T20260505-6]** — Replace timestamp-only command-audit execution ids with collision-resistant generated ids for parallel tool runs.
- **[T20260506-2]** — Lazily materialize loop audit JSONL files only when loop-level events are emitted.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
