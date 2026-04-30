# Auditability — Decisions

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-04-30 (T20260430-5)

This is the append-only ADR log for Auditability. Entries are ordered by ADR number. New entries should use the template in [../CONVENTIONS.md](../CONVENTIONS.md) and cite the task that made the decision real.

---

## ADR-001 — Dedicated auditability design ownership

**Status:** Accepted · 2026-04 · [T20260426-0605]

**Context.** Auditability is named as a primary Orbit feature, but the implementation and rationale were spread across README prose, Activity / Job docs, SQLite audit code, loop audit code, and redaction utilities.

**Decision.** Create `docs/design/auditability/` as the canonical design folder for auditability, with codex as owner.

**Consequences.**
- Audit decisions now have one ADR log and one glossary.
- Future audit coverage work can cite a feature-owned spec rather than copying README promises.
- Cost: auditability now overlaps with Activity / Job docs, so cross-links must stay current rather than duplicating the full v2 runtime design.

## ADR-002 — Command audit rows stay compact and queryable

**Status:** Accepted · 2026-04 · [T20260426-0605]

**Context.** CLI commands need a durable, filterable history across process invocations, but stuffing full provider payloads into command rows would make routine audit queries noisy and expensive.

**Decision.** Keep command audit records as compact SQLite rows with command, target, role, status, timing, working directory, and optional argument/error fields. Store transcript-level detail in run-trace JSONL and blobs instead.

**Consequences.**
- `orbit audit list/show/stats/export` can stay fast and table-shaped.
- Full replay data has a separate home better suited to append-only files and content-addressed blobs.
- Cost: reconstructing a complete incident can require joining command rows with job state and file-backed traces.

## ADR-003 — V2 run structure and loop transcript detail are separate audit layers

**Status:** Accepted · 2026-04 · [T20260419-0002]

**Context.** Activity/job execution needs run, step, retry, fan-out, loop, and activity structure. Provider loops need HTTP, tool-call, payload, and session detail. One event type cannot serve both needs cleanly.

**Decision.** Use `V2AuditEnvelope` for activity/job structure and keep `LoopAuditEvent` for provider/tool detail. Connect the layers through run ids and parent event ids rather than merging them into one schema.

**Consequences.**
- Workflow replay can traverse a run tree without loading every provider payload.
- Loop-level audit can evolve with provider/tool semantics without changing the job DAG envelope.
- Cost: reviewers need tooling or documentation to move between related files.

## ADR-004 — File-backed run traces are workspace-local state

**Status:** Accepted · 2026-04 · [T20260426-0519]

**Context.** V2 JSONL and blob traces were runtime artifacts, but they previously lived under a first-level `.orbit/audit/` path that blurred command audit, workspace state, and durable authoring surfaces.

**Decision.** Store activity/job envelopes, loop events, and blobs under `.orbit/state/audit/`, while command audit rows remain in the configured SQLite audit database.

**Consequences.**
- Runtime traces live with other workspace-local run state.
- The file layout distinguishes command audit queries from run reconstruction artifacts.
- Cost: old local `.orbit/audit/` artifacts may require manual fallback or migration if a user wants historical run reconstruction.

## ADR-005 — Redaction is a write-side durability boundary

**Status:** Accepted · 2026-04 · [T20260426-0605]

**Context.** Audit needs faithful payloads for reproducibility, but storing raw provider keys or sensitive environment-derived values would make the audit trail unsafe by default.

**Decision.** Redact sensitive env values, HTTP authorization patterns, API-key fields, bearer tokens, and selected argv token shapes before durable blob or error-message persistence.

**Consequences.**
- Audit readers can treat normal stored blobs as already redacted.
- Smoke tests can verify stored bytes, not just display output.
- Cost: redaction changes payload hashes and may remove exact bytes that would otherwise help reproduce a provider interaction.

## ADR-006 — Invocation metrics are audit-adjacent primary records

**Status:** Accepted · 2026-04 · [T20260426-0526]

**Context.** V2 job execution emitted audit JSONL, but metrics and scoreboards read the invocation store. Deriving metrics by scraping audit logs would couple operator reporting to provider transcript format and JSONL retention.

**Decision.** Persist `InvocationTrace` records beside audit as first-class metric records keyed by job run, activity, task ids, agent, model, usage, and tool-call summaries.

**Consequences.**
- `orbit metrics` and scoreboards do not need to parse audit JSONL.
- CLI and HTTP agent-loop paths can converge on one usage record shape.
- Cost: job execution has another persistence side effect, and metrics can diverge from transcript detail if a provider path reports incomplete usage.

## ADR-007 — Run trace inspection stays separate from command audit

**Status:** Accepted · 2026-04 · [T20260426-0705], [T20260426-0709]

**Context.** Operators need first-class commands for activity/job envelope JSONL, but `orbit audit` is intentionally the compact SQLite command-audit surface. Mixing run-local envelope traversal into command-audit queries would blur two storage scopes and make command audit rows carry workflow-specific semantics.

**Decision.** Expose v2 envelope inspection under `orbit run events` and `orbit run trace`, and keep `orbit audit` focused on command-audit rows. Keep envelope JSONL/blob parsing behind orbit-core runtime accessors so CLI rendering does not own file-backed run-trace layout.

**Consequences.**
- Operators can inspect both command history and run-local workflow traces through dedicated commands.
- Activity `step.id` becomes the shared selector for `run show`, `run logs`, and run trace/event inspection.
- Cost: users must understand that `orbit audit` and `orbit run events/trace` answer related but different audit questions.

## ADR-008 — Process tracing feed is global JSONL

**Status:** Accepted · 2026-04 · [T20260426-2343]

**Context.** CLI subprocess output now emits structured tracing events after [T20260426-2313], but subscriber initialization happens before Orbit resolves a workspace root.

**Decision.** Append process-level tracing events to `~/.orbit/state/logs/orbit.jsonl` through the default subscriber, using the same `EnvFilter` as stderr and a non-blocking file writer retained for the process lifetime.

**Consequences.**
- Operators and future dashboards can tail one machine-readable feed across workspaces.
- Early bootstrap events have a durable destination without needing runtime path resolution.
- Cost: the v1 file is unrotated and concurrent processes can rarely interleave oversized JSONL records.

## ADR-009 — Tracing redaction is enforced by field formatting

**Status:** Accepted · 2026-04 · [T20260426-2349]

**Context.** The global JSONL feed made `tracing` output durable, but pre-emission helpers such as `redact_event_text` only protected call sites that remembered to use them.

**Decision.** Install a redacting `FormatFields` implementation on both stderr and JSONL tracing formatters. The formatter redacts string field values, `Debug`-formatted field values, and unstructured `message` output while preserving field names and typed numeric/boolean JSON values.

**Consequences.**
- New structured tracing emitters inherit the default redaction path before data reaches terminal or disk output.
- CLI subprocess tracing can emit raw line fields while the retained stdout/stderr audit blobs preserve original bytes.
- Cost: span attribute redaction, binary payload redaction, and user-configurable redaction policies remain separate follow-up concerns.

## ADR-010 — Canonical audit stores project high-signal events to tracing

**Status:** Accepted · 2026-04 · [T20260427-0023]

**Context.** The global JSONL tracing feed existed, but policy denials and friction submissions still only reached their canonical stores or return paths. Operators tailing the live feed could miss the highest-signal safety and agent-friction events.

**Decision.** Emit structured `tracing::warn!` projections beside the existing canonical side effects for filesystem policy denials, proc-spawn allowlist denials, and friction task submissions. Keep the SQLite audit rows, FS audit events, `OrbitError::PolicyDenied` returns, and scoreboard updates authoritative.

**Consequences.**
- Dashboards and operators can watch `orbit.policy.deny` and `orbit.friction.reported` without querying the canonical stores.
- New producers can follow the same dual-write pattern: persist to the source of truth first, then project a redacted live event.
- Cost: the tracing feed is lossy and filterable, so readers must not treat missing live events as proof that the canonical store has no matching record.

## ADR-011 — Unified log feed: producer completion + reader CLI

**Status:** Accepted · 2026-04 · [T20260427-27]

**Context.** Producer-side coverage of the unified JSONL tracing feed (ADR-008/009/010) reached policy and friction events, but three gaps blocked the v2-terminal-console mockup from being demonstrable end-to-end:
1. Job-DAG lifecycle events (`step.*`, `fanout.*`, `worker.state`, `loop.*`) flowed only into the `V2AuditWriter` audit store.
2. ~16 stray `eprintln!` / `println!` calls in library crates (orbit-core, orbit-engine, orbit-store, orbit-knowledge) bypassed `tracing` entirely.
3. Operators had to `tail -F | jq` the JSONL file by hand — no first-class reader existed.

**Decision.** Close all three slices in one task:
- **Slice 1.** Add a single `emit_job_event` dual-write helper in `crates/orbit-engine/src/activity_job/job_executor.rs` that pairs every `V2AuditEventKind` lifecycle emission with a structured `tracing::*!` event under stable targets (`orbit.job.step_started`, `orbit.job.step_finished`, `orbit.job.step_skipped`, `orbit.job.step_retry`, `orbit.job.step_denied`, `orbit.job.step_join`, `orbit.job.fanout`, `orbit.job.worker_state`, `orbit.job.loop_iteration`, `orbit.job.loop_did_not_converge`). Tracing emit precedes the audit emit so the live feed reflects activity before the audit lock; the audit store remains authoritative. The helper is the only call site that pairs both writes — adding a new variant only requires touching `emit_job_tracing`.
- **Slice 2.** Migrate every stray `eprintln!` / `println!` in library crates to `tracing::warn!` / `error!` / `info!` with namespaced targets (`orbit.store.sqlite`, `orbit.task.dependencies`, `orbit.knowledge.*`, etc.) and stable structured fields. Add `#![deny(clippy::print_stderr, clippy::print_stdout)]` to every library crate root (`orbit-common`, `orbit-policy`, `orbit-exec`, `orbit-knowledge`, `orbit-store`, `orbit-tools`, `orbit-agent`, `orbit-engine`, `orbit-core`, `orbit-mcp`); `orbit-cli` and `examples/` stay exempt because they own user-facing stdout/stderr. `cargo clippy --workspace --all-targets -- -D warnings` is the regression-prevention gate.
- **Slice 3.** Add `orbit log tail` at `crates/orbit-cli/src/command/log/`. The reader resolves the JSONL path from `--path` → `$ORBIT_LOG_PATH` → `orbit_common::utility::logging::global_jsonl_log_path()` (newly made `pub`). It filters by `--target` prefix, `--level` minimum, and `--since` duration; emits raw lines under `--json` and a four-column rendering otherwise (timestamp · source · code · message) with target-aware formatters for the high-signal targets above plus the cli_runner subprocess target. ANSI escapes are suppressed when stdout is not a TTY. Follow mode (`-f`) seeks to EOF after the initial window and polls every 50 ms.

**Consequences.**
- The v2-terminal-console mockup is fully demonstrable from real Orbit binaries; no fictional events.
- Library code can no longer regress on `eprintln!`/`println!` — clippy fails the workspace under `-D warnings`.
- The audit store and JSONL feed stay independent: schema and bytes-level shape of `V2AuditEnvelope` records are unchanged, the tracing feed is purely additive.
- Cost: scheduler-event semantics from the mockup remain aspirational (Orbit has no scheduler), follow mode does not handle file rotation or truncation, and the CLI reader currently keeps the entire file in memory before applying `-n` (acceptable for the v1 unrotated file).

## ADR-012 — Friction scorekeeping derives from lifecycle history

**Status:** Accepted · 2026-04 · [T20260427-43]

**Context.** Friction reports already used `type: friction`, but untriaged reports shared `status: proposed` with human-authored proposals. That made the friction bounty scoreboard depend on ambiguous proposed-state transitions and made MCP filing harder to express.

**Decision.** Add `status: friction` as the creation status for agent self-reported friction, infer the paired type/status at creation, and rebuild `friction_bounty.json` from task history. Reported counts come from `type: friction`; accepted and rejected counts come only from exits out of `status: friction`.

**Consequences.**
- Friction inbox items are separated from human proposals without losing the lifetime `type: friction` category.
- Scoreboard refreshes can repair stale increment files because task history is the source of truth for triage outcomes.
- Cost: legacy untriaged friction tasks need a file-store migration from `proposed/` to `friction/`, and already-triaged legacy histories remain dependent on their existing transition records.

## ADR-013 — Unified log feed exposes shared backend surfaces for dashboard UI

**Status:** Accepted · 2026-04 · [T20260427-44], [T20260427-46]

**Context.** ADR-011 made `orbit log tail` the first-class terminal reader for the global JSONL tracing feed. The dashboard needs the same source/code/message semantics without copying formatter logic into browser JavaScript, and the UI feature is owned separately from the backend/API slice.

**Decision.** Extract the CLI tail formatter/filter/path-resolution logic into a shared `orbit-cli` log module and expose two read-only dashboard backend endpoints: `/api/log` for a bounded initial snapshot and `/api/log/stream` for Server-Sent Events from newly appended JSONL lines. Both endpoints resolve the same log path as `orbit log tail`, accept the same target/level/since filters, and render `message_html` server-side with dynamic field values HTML-escaped before adding emphasis markup. The Tasks-tab DOM/CSS/JS panel remains a Gemini-owned follow-up task that consumes this API contract rather than duplicating log semantics.

**Consequences.**
- CLI, dashboard backend, and dashboard UI share one log vocabulary for source labels, short codes, level filtering, and high-value target messages.
- Browser clients receive pre-rendered, escaped message HTML while keeping dynamic labels and classes outside formatter templates.
- Cost: the backend stream still follows the v1 append-only file model; rotation/truncation handling is best-effort and the visual panel ships separately under the UI-owned task.

## ADR-014 — Tool-call provenance is model-first

**Status:** Accepted · 2026-04 · [T20260427-52]

**Context.** Orbit task and workflow instructions told agents to provide both `agent` and `model` in every `orbit tool run` JSON payload. That duplicated information for the built-in providers and created a mismatch class where an exact model could be paired with the wrong agent family.

**Decision.** Deprecate `agent` as a normal tool-call input and make exact `model` the preferred provenance field. Tool dispatch infers the agent family from known model names, persists both fields internally for compatibility and scoreboards, and rejects explicit legacy `agent` values when they contradict an inferable model family.

**Consequences.**
- Seeded skills and instructions can show shorter model-only tool calls while task records still retain `agent` and `model`.
- Legacy callers that still pass `agent` continue to work when the pair is consistent.
- Cost: unknown or ambiguous model names cannot infer an agent family; callers that need family-specific dispatch for those names must still provide a compatible legacy `agent` value.

## ADR-015 — Task attribution can be corrected explicitly

**Status:** Accepted · 2026-04 · [T20260427-47]

**Context.** Automatic task attribution keeps routine lifecycle updates low-friction, but it can leave stale `planned_by` or `implemented_by` values when a task is started by one actor and finished by another.

**Decision.** Keep automatic stamping for plan writes and review/done transitions, but let task update callers provide explicit `planned_by` and `implemented_by` values. Explicit attribution values override automatic stamps within the same update, and empty strings clear the corresponding field.

**Consequences.**
- Agents can correct split or stale task provenance without editing task files directly.
- Existing lifecycle automation keeps working when explicit attribution fields are omitted.
- Cost: attribution fields become intentionally editable metadata, so reviewers must read task history and audit rows when they need stronger authorship evidence than the latest field value.

## ADR-016 — Tool-invocation audit is owned by the runtime, with MCP preflight bracketing

**Status:** Accepted · 2026-04 · [T20260428-4]

**Context.** Tool-invocation audit was historically written by `AuditGuard` in `orbit-cli`, an RAII wrapper around CLI command dispatch. MCP `tools/call` requests entered through `orbit mcp serve`, which executes outside any `AuditGuard`, and called `OrbitRuntime::execute_tool_command` directly. The runtime emitted only an in-memory `OrbitEvent::ToolExecuted`, so MCP-originated tool calls were missing from the SQLite command-audit trail entirely. A second gap sat at the MCP preflight check (`ensure_mcp_tool_exposed` in `orbit-cli/src/command/mcp/mod.rs`): unknown or unexposed tool names were rejected before runtime dispatch, so even a runtime-level audit-write would not cover the failure path that the acceptance criteria for the fix called out as the sharpest case.

**Decision.** Move tool-invocation audit recording into the runtime layer (`OrbitRuntime::execute_tool_command_dispatch`), so every entry point — CLI tool-run, MCP, future HTTP — produces an audit row from a single seam. Tag each dispatch with a `ToolEntryPoint` discriminator (`Cli`, `Mcp`) encoded in the audit `subcommand` field (`"run"` vs `"run-mcp"`) to avoid an audit-table schema migration. Bracket the MCP path with `audited_mcp_call`, which records a failure-status audit row when preflight rejects an unknown or unexposed tool name and otherwise delegates to the runtime for the dispatch audit. To prevent the legacy CLI `AuditGuard` from double-emitting on the `orbit tool run` path, expose a per-thread `mark_tool_audit_recorded` / `take_tool_audit_recorded` signal that the runtime sets after writing and the guard checks during `Drop`. CLI paths that bail before the runtime is reached (invalid JSON, missing input, `--dry-run`) leave the signal clear and continue to produce their existing guard-side audit row.

**Consequences.**
- MCP-originated tool calls now show up in `orbit audit list` with full agent/model identity resolved from the same input-JSON → CLI-flags → env-vars precedence the CLI uses.
- Preflight failures for unknown / unexposed tool names are auditable in their own right; the failure layer is no longer silent.
- Adding another entry point (HTTP, IPC) can reuse `execute_tool_command_dispatch` without re-implementing audit-write.
- `duration_ms` is clamped to `>= 1` at the audit-write site so sub-millisecond invocations cannot record `0` and false-trigger downstream alerts that key on `duration_ms > 0`.
- Cost: the dedup signal is a per-thread one-shot. Async or multi-threaded entry points that span thread boundaries between dispatch and the higher-level audit writer will need to re-evaluate this seam if they emerge; today, the CLI is sync and `orbit mcp serve` runs without a CLI guard at all, so the constraint is a non-issue.

## ADR-017 — Command-audit rows carry task / run / activity correlation IDs

**Status:** Accepted · 2026-04 · [T20260428-7]

**Context.** The SQLite `audit_events` table records every CLI- and MCP-originated tool invocation but stored no link back to the Orbit task, job run, or activity that triggered it. The dashboard's command-audit view rendered rows like `claude-opus-4-7 → orbit.task.approve → success` with no way to drill back to the surrounding execution context. The v2 audit envelope (`V2AuditEnvelope`, ADR-003) already carries `run_id`, `parent_event_id`, and activity context as workspace-local JSONL, but the two streams were unjoined: command-audit rows had no foreign-key into v2 events, and v2 events had no `execution_id` back-reference to the SQLite row. Operator drilldown stopped at the row.

**Decision.** Add four nullable correlation columns to `audit_events` — `task_id`, `job_run_id`, `activity_id`, `step_index` — and populate them at the runtime tool-dispatch seam established in ADR-016. The runtime resolves each field with the same precedence used for agent/model identity: caller-asserted value from the tool input JSON wins, falling back to the runtime-asserted env vars (`ORBIT_TASK_ID`, `ORBIT_RUN_ID`, `ORBIT_ACTIVITY_ID`, `ORBIT_STEP_INDEX`) exported by the engine when it spawned the agent subprocess. The engine's `state_env_vars` is extended to emit `ORBIT_TASK_ID` (sourced from activity input by the same convention as `execution_working_directory_with_task`) and `ORBIT_ACTIVITY_ID` (sourced from `execution.activity.id`) alongside the existing `ORBIT_RUN_ID`. Indexes on `(task_id)` and `(job_run_id)` keep correlation queries cheap. The dashboard surfaces the four fields in the audit detail row and renders `job_run_id` as a deep link to the existing `#runs/<id>` view.

**Consequences.**
- An operator clicking through a command-audit row can immediately see "this `orbit.task.approve` ran under task `T...` inside run `jrun-...` step `2`," and the run id is one click away from the run-detail page.
- Failure triage for MCP tool calls — denials, validation failures, sandbox rejections — gains the surrounding context without out-of-band correlation.
- The two audit streams (SQLite command rows + workspace-local v2 envelope) remain separate, consistent with ADR-003. ADR-002's "compact rows" principle is preserved: the new columns are short identifiers, not transcript payloads.
- Trust boundary: input-JSON values can be asserted by an MCP client and should be treated as caller-claimed; env-supplied values are the engine's ground truth. Code comments on `resolve_audit_context` document the precedence so future call sites do not invert it.
- Cost: a one-time SQLite migration (`ALTER TABLE audit_events ADD COLUMN ...`). Historical rows render with NULL correlation cells; backfill is intentionally out of scope.

## ADR-018 — Scoreboard tool-call totals project from command audit

**Status:** Accepted · 2026-04 · [T20260428-11]

**Context.** `summary.json` previously sourced `tool_calls` from the token/invocation scoreboard, which can be empty or zero for providers that do not emit invocation traces. At the same time, the SQLite command-audit trail now records every CLI and MCP tool-run attempt, including runtime failures, pre-runtime CLI failures, and denials.

**Decision.** Treat command-audit rows as the source for scoreboard all/failed tool-run attempt counts. `summary.json` counts `command: tool` rows whose `subcommand` is `"run"` or `"run-mcp"` and whose `tool_name` is present, groups them by normalized role/model, writes the all-attempt count to the existing `tool_calls` field, and adds `failed_tool_calls` for non-success rows (`failure` and `denied`). Token totals continue to come from the token/invocation scoreboard. Legacy token-scoreboard `total_tool_calls` remains a fallback through a max overlay instead of being added to audit counts.

**Consequences.**
- Failed and denied tool runs become visible in the compact scoreboard summary rather than only in `orbit audit list`.
- Non-Claude or trace-sparse providers can still show tool activity because command audit does not depend on provider usage traces.
- The existing `tool_calls` JSON field keeps its name and now represents all known tool-run attempts rather than only invocation-trace tool-call summaries.
- Cost: the max overlay is conservative, not a perfect dedupe. If invocation traces contain tool calls that are genuinely absent from command audit while audit rows also exist for the same model, the summary may undercount the mathematical union until both streams share a common invocation id.

## ADR-019 — Task-review feedback scores separately from PR review comments

**Status:** Accepted · 2026-04 · [T20260428-17]

**Context.** Orbit task review threads are the local review surface, while GitHub PR review comments are an external PR workflow artifact. Counting local-only review-thread messages directly in `pr.review_comments` would make the PR scoreboard ambiguous for tasks that never opened or synced a pull request.

**Decision.** Keep `pr.review_comments` limited to comments that enter the PR/GitHub review flow, including Orbit review-thread messages after successful GitHub sync. Score local Orbit review-thread messages in a separate `task-review-messages` metric stored in `task_review.json` and surfaced in compact summaries as `task_review.messages`. Both local task-review scoring and GitHub sync scoring require the message model label to resolve to an exact configured orchestrator/helper model from the active executor catalog, falling back to Orbit's built-in `resolve_agent_model_pair` defaults when no host-specific model pair exists. Prefix-only family inference such as `gpt-typo` or `opus-handle` is not a scoring identity.

**Consequences.**
- Local code-review feedback earns scoreboard credit immediately when a scored agent creates or replies to an Orbit task review thread.
- The dashboard can show local review feedback beside PR review feedback without renaming or overloading the existing PR fields.
- A review-thread message that is created locally and later synced to GitHub can appear once in `task_review.messages` and once in `pr.review_comments`; those are intentionally distinct workflow metrics, not one mixed counter.
- `summary.json` schema version 2 adds `task_review.messages`; consumers that only understand schema version 1 can ignore the additional field.
- Cost: review productivity now has two counters. Readers need to compare both fields for a full picture of review activity, and future aggregate views must avoid adding them together without a clear label.

---

## Task References

- **[T20260419-0002]** — Add workspace provenance and v2 audit envelope events for activity/job execution.
- **[T20260426-0519]** — Move file-backed activity/job audit traces under workspace state.
- **[T20260426-0526]** — Persist v2 invocation traces for metrics beside audit.
- **[T20260426-0605]** — Add this auditability design folder and record initial ADRs.
- **[T20260426-0705]** — Expose v2 run audit events through `orbit run events` and `orbit run trace`.
- **[T20260426-0709]** — Align run step selectors on activity `step.id` and move CLI invocation log reading behind orbit-core runtime accessors.
- **[T20260426-2313]** — Stream CLI subprocess stdout/stderr through structured tracing events.
- **[T20260426-2343]** — Add the global process tracing JSONL feed at `~/.orbit/state/logs/orbit.jsonl`.
- **[T20260426-2349]** — Apply tracing-layer redaction before stderr and global JSONL output.
- **[T20260427-0023]** — Project policy denials and friction task submissions into the global tracing feed.
- **[T20260427-27]** — Close out the unified-log story: job lifecycle dual-write, library print migration with workspace lint gate, and `orbit log tail` reader CLI.
- **[T20260427-43]** — Add `status: friction`, creation-time type/status inference, migration, and history-derived friction bounty refresh.
- **[T20260427-44]** — Add shared log formatter extraction and dashboard backend `/api/log` snapshot/SSE endpoints.
- **[T20260427-46]** — Implement the Gemini-owned Tasks-tab `orbit.log` panel using the shared dashboard backend API.
- **[T20260427-47]** — Allow explicit task attribution correction for `planned_by` and `implemented_by` through task update paths.
- **[T20260427-52]** — Deprecate `agent` in normal tool-call JSON, infer agent family from `model`, and reject inconsistent legacy pairs.
- **[T20260428-4]** — Record audit events for MCP tool invocations: move tool-invocation audit into the runtime, add the `ToolEntryPoint` discriminator, and bracket MCP preflight + dispatch in `audited_mcp_call`.
- **[T20260428-7]** — Correlate command-audit rows with originating run/task/activity: add nullable `task_id` / `job_run_id` / `activity_id` / `step_index` columns, thread context through engine env vars, populate at the runtime dispatch seam, surface on the dashboard.
- **[T20260428-11]** — Derive compact scoreboard all/failed tool-call counts from command-audit tool-run rows.
- **[T20260428-17]** — Split local Orbit task-review scoring from PR review-comment scoring and surface both in compact scoreboards.
- **[T20260430-5]** — Tighten task and PR review-message scoring so only exact configured orchestrator/helper model identities score; typo-prefixed labels are ignored.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
