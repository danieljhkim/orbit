---
summary: "Auditability — Decisions"
type: design
title: "Auditability — Decisions"
owner: codex
last_updated: 2026-08-02
last_validated: 2026-07-27
status: Draft
feature: auditability
doc_role: decisions
tags: ["auditability"]
---

# Auditability — Decisions

This is the append-only ADR log for Auditability. Entries are ordered by ADR number. New entries should use the template in [../CONVENTIONS.md](../CONVENTIONS.md) and cite the task that made the decision real.

Historical note ([ORB-10458]): the entries listed below were authored with local IDs that had no record in the ADR store. They were allocated through `orbit.adr.add`, their narratives migrated into the store verbatim, and their headings rewritten to the allocated global ID. The original local IDs survive as `legacy_ids`, so prior citations still resolve via `orbit tool run orbit.adr.show --input '{"legacy_id":"<feature>/ADR-NNN"}'`. Backfilled here: `auditability/ADR-012` → ADR-0278, `auditability/ADR-022` → ADR-0279, `auditability/ADR-023` → ADR-0280.

---

## ADR-001 — Dedicated auditability design ownership

**Status:** Accepted · 2026-04 · [T20260426-0605]

**Context.** Auditability is a primary Orbit feature, but its implementation and rationale were spread across README prose, Activity / Job docs, SQLite audit code, loop audit code, and redaction utilities.

**Decision.** Create `docs/design/auditability/` as the canonical auditability design folder, owned by codex.

**Consequences.**
- Audit decisions now have one ADR log and one glossary.
- Cost: auditability overlaps with Activity / Job docs, so cross-links must stay current instead of duplicating the full runtime design.

## ADR-002 — Command audit rows stay compact and queryable

**Status:** Accepted · 2026-04 · [T20260426-0605]

**Context.** CLI commands need durable, filterable history across processes, but full provider payloads would make routine queries noisy and expensive.

**Decision.** Keep command audit records as compact SQLite rows with command, target, role, status, timing, working directory, and optional argument/error fields; store transcript detail in JSONL and blobs.

**Consequences.**
- `orbit audit list/show/stats/export` can stay fast and table-shaped.
- Cost: complete incident reconstruction may require joining command rows with job state and file-backed traces.

## ADR-003 — V2 run structure and loop transcript detail are separate audit layers

**Status:** Accepted · 2026-04 · [T20260419-0002]

**Context.** Activity/job execution needs run, step, retry, fan-out, loop, and activity structure. Provider loops need HTTP, tool-call, payload, and session detail.

**Decision.** Use `V2AuditEnvelope` for activity/job structure and `LoopAuditEvent` for provider/tool detail, connected through run ids and parent event ids.

**Consequences.**
- Workflow replay can traverse a run tree without loading every provider payload.
- Cost: reviewers need tooling or documentation to move between related files.

## ADR-004 — File-backed run traces are workspace-local state

**Status:** Accepted · 2026-04 · [T20260426-0519]

**Context.** V2 JSONL and blob traces are runtime artifacts, but their old first-level `.orbit/audit/` path blurred command audit, workspace state, and authored docs.

**Decision.** Store activity/job envelopes, loop events, and blobs under `.orbit/state/audit/`; keep command audit rows in the configured SQLite database.

**Consequences.**
- Runtime traces live with other workspace-local run state.
- Cost: old `.orbit/audit/` artifacts may need manual fallback or migration for historical reconstruction.

## ADR-005 — Redaction is a write-side durability boundary

**Status:** Accepted · 2026-04 · [T20260426-0605]

**Context.** Audit needs useful payloads for reproducibility, but raw provider keys or sensitive environment-derived values would make the trail unsafe by default.

**Decision.** Redact sensitive environment values, HTTP authorization patterns, API-key fields, bearer tokens, and selected argv token shapes before durable blob or error-message persistence.

**Consequences.**
- Audit readers can treat normal stored blobs as already redacted.
- Cost: redaction changes payload hashes and may remove exact bytes useful for reproducing a provider interaction.

## ADR-006 — Invocation metrics are audit-adjacent primary records

**Status:** Accepted · 2026-04 · [T20260426-0526]

**Context.** V2 job execution emits audit JSONL, but metrics and scoreboards read the invocation store. Scraping audit logs would couple reporting to transcript format and retention.

**Decision.** Persist `InvocationTrace` records beside audit as first-class metric records keyed by job run, activity, task ids, agent, model, usage, and tool-call summaries.

**Consequences.**
- Dashboard metrics endpoints and scoreboards can avoid parsing audit JSONL.
- Cost: metrics can diverge from transcript detail if a provider path reports incomplete usage.

## ADR-0173 — Dashboard owns invocation metrics surfaces

**Status:** Accepted · 2026-05 · [ORB-00190]

**Context.** The metrics CLI surface is unused, and [ORB-00191] moved the missing knowledge, activity, tool, task, and invocation views into dashboard HTTP endpoints. Keeping a second JSON-capable command would make future metrics work maintain two surfaces.

**Decision.** The dashboard is the canonical user-facing and programmatic surface for invocation metrics. The metrics CLI command is retired, and future observability features should ship as dashboard endpoints and views.

**Consequences.**
- Programmatic consumers use the dashboard HTTP API (`/api/metrics/*`) instead of a dedicated CLI JSON scripting surface.
- Future invocation-metrics features build as dashboard endpoints first.
- No single code anchor; this convention is enforced through design docs and review.
- Cost: shell scripts cannot rely on a dedicated metrics command and must call the local dashboard API or shared runtime libraries.

## ADR-007 — Run trace inspection stays separate from command audit

**Status:** Accepted · 2026-04 · [T20260426-0705], [T20260426-0709]

**Context.** Operators need first-class commands for activity/job envelope JSONL, but `orbit audit` is the compact SQLite command-audit surface.

**Decision.** Expose v2 envelope inspection under `orbit run events` and `orbit run trace`, and keep envelope/blob parsing behind orbit-core runtime accessors.

**Consequences.**
- Command history and run-local workflow traces have dedicated commands.
- Cost: users must learn that `orbit audit` and `orbit run events/trace` answer related but different questions.

## ADR-008 — Process tracing feed is global JSONL

**Status:** Accepted · 2026-04 · [T20260426-2343]

**Context.** CLI subprocess output emits structured tracing events after [T20260426-2313], but subscriber initialization happens before Orbit resolves a workspace root.

**Decision.** Append process-level tracing events to `~/.orbit/state/logs/orbit.jsonl` through the default subscriber using the same `EnvFilter` as stderr and a retained non-blocking writer.

**Consequences.**
- Operators and dashboards can tail one machine-readable feed across workspaces.
- Cost: the v1 file is unrotated and concurrent processes can rarely interleave oversized JSONL records.

## ADR-009 — Tracing redaction is enforced by field formatting

**Status:** Accepted · 2026-04 · [T20260426-2349]

**Context.** A durable JSONL feed made tracing output persistent, but call-site helpers only protected emitters that remembered to use them.

**Decision.** Install redacting `FormatFields` implementations on stderr and JSONL tracing formatters so string fields, `Debug` values, and messages are scrubbed before output.

**Consequences.**
- New structured tracing emitters inherit default redaction before terminal or disk output.
- Cost: span attribute redaction, binary payload redaction, and user-configurable policies remain follow-up concerns.

## ADR-010 — Canonical audit stores project high-signal events to tracing

**Status:** Accepted · 2026-04 · [T20260427-0023]

**Context.** Policy denials and friction submissions reached canonical stores or return paths, but operators tailing the live feed could miss them.

**Decision.** Emit structured `tracing::warn!` projections beside canonical side effects for filesystem denials, proc-spawn denials, and friction task submissions.

**Consequences.**
- Dashboards can watch `orbit.policy.deny` and `orbit.friction.reported` without querying canonical stores.
- Cost: the tracing feed is lossy and filterable, so missing live events cannot prove the canonical store has no matching record.

## ADR-011 — Unified log feed: producer completion + reader CLI

**Status:** Accepted · 2026-04 · [T20260427-27]

**Context.** The unified JSONL feed still lacked job-DAG lifecycle projections, library print hygiene, and a first-class reader for the v2-terminal-console mockup.

**Decision.** Add one `emit_job_event` dual-write helper for job lifecycle tracing, migrate library `println!`/`eprintln!` calls to structured tracing with clippy denies in library crates, and add `orbit log tail` with path, target, level, since, follow, and JSON options.

**Consequences.**
- The terminal-console mockup can use real Orbit events, and library crates fail clippy if raw prints return.
- Cost: scheduler-event semantics remain aspirational, follow mode is v1, and the reader keeps the file in memory before applying `-n`.

## ADR-0278 — Friction scorekeeping derives from lifecycle history

**Status:** Superseded · 2026-05 · [T20260510-13] · legacy_id: `auditability/ADR-012`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0278"}'`.

## ADR-013 — Unified log feed exposes shared backend surfaces for dashboard UI

**Status:** Accepted · 2026-04 · [T20260427-44], [T20260427-46]

**Context.** `orbit log tail` established terminal semantics, but the dashboard needed the same source/code/message vocabulary without copying formatter logic into browser JavaScript.

**Decision.** Extract log formatter/filter/path logic into a shared `orbit-cli` module and expose dashboard `/api/log` snapshot plus `/api/log/stream` SSE endpoints that render escaped `message_html` server-side.

**Consequences.**
- CLI, dashboard backend, and dashboard UI share one log vocabulary and escaping boundary.
- Cost: stream rotation/truncation handling is best-effort, and the visual panel ships separately under UI ownership.

## ADR-014 — Tool-call provenance was model-first

**Status:** Superseded by [agent-families ADR-0154](../agent-families/4_decisions.md#adr-0154--collapse-agent-identity-to-family-and-move-model-strings-to-configuration) · 2026-05 · [ORB-00080]

**Context.** Asking agents to pass both `agent` and `model` duplicated information and allowed exact models to be paired with the wrong family.

**Decision.** Originally deprecated `agent` as a normal tool-call input and used `model` for provenance. [Agent-families ADR-0154](../agent-families/4_decisions.md#adr-0154--collapse-agent-identity-to-family-and-move-model-strings-to-configuration) superseded the exact-model convention: `model` now carries the canonical agent family, with full model strings accepted only as compatibility input that normalizes to family.

**Consequences.**
- Seeded skills and instructions still use a single `model` provenance field, but examples teach family values (`codex`, `claude`, `gemini`, `grok`).
- Cost: compatibility normalization must remain for historical full-model inputs and external callers that have not migrated yet.

## ADR-015 — Task attribution can be corrected explicitly

**Status:** Accepted · 2026-04 · [T20260427-47]

**Context.** Automatic task attribution is low-friction but can leave stale `planned_by` or `implemented_by` values when different actors start and finish work.

**Decision.** Keep automatic stamping for plan writes and review/done transitions, but let task update callers explicitly set or clear `planned_by` and `implemented_by`.

**Consequences.**
- Agents can correct split or stale provenance without editing task files directly.
- Cost: attribution fields are editable metadata, so stronger authorship evidence still requires task history and audit rows.

## ADR-016 — Tool-invocation audit is owned by the runtime, with MCP preflight bracketing

**Status:** Accepted · 2026-04 · [T20260428-4]

**Context.** CLI `AuditGuard` historically wrote tool-invocation audit rows, leaving MCP `tools/call` dispatch and MCP preflight failures outside the SQLite command-audit trail.

**Decision.** Move tool-invocation audit to `OrbitRuntime::execute_tool_command_dispatch`, tag dispatches as CLI `"run"` or MCP `"run-mcp"`, bracket MCP preflight failures in `audited_mcp_call`, and use a per-thread signal so CLI guard rows are not duplicated.

**Consequences.**
- CLI and MCP tool calls, including unknown/unexposed MCP failures, now produce one audit row with shared identity resolution.
- Cost: the dedup signal is thread-local; future async or cross-thread guarded entry points must re-evaluate the boundary.

## ADR-017 — Command-audit rows carry task / run / activity correlation IDs

**Status:** Accepted for CLI; MCP precedence superseded · 2026-07 · [ORB-10228]

**Context.** SQLite command-audit rows recorded tool invocations but had no direct link to the task, job run, activity, or step that caused them.

**Decision.** Add nullable `task_id`, `job_run_id`, `activity_id`, and `step_index` columns. CLI retains caller-JSON-first compatibility. For MCP, [ORB-10228] explicitly supersedes that precedence: caller JSON is never trusted audit correlation; an authenticated managed envelope supplies task/run/activity/step, and optional trusted `leased_run.run_id` may fill or must match canonical `job_run_id`.

**Consequences.**
- Operators can drill from a tool row to the originating task and run context without out-of-band correlation.
- MCP standalone calls keep these fields NULL unless trusted lease correlation supplies `job_run_id`; managed envelope identity wins over client claims.
- Cost: historical rows remain NULL, and CLI caller-asserted JSON remains weaker evidence than engine-supplied or MCP trusted context.

## ADR-018 — Scoreboard tool-call totals project from command audit

**Status:** Accepted · 2026-04 · [T20260428-11]

**Context.** `summary.json` used token/invocation scoreboard tool-call totals, which can be empty for providers that do not emit invocation traces, while command audit records every tool-run attempt.

**Decision.** Count `command: tool` rows with `subcommand: "run"` or `"run-mcp"` and `tool_name` present as scoreboard all/failed tool-run attempts; keep token totals sourced from invocation/token scoreboards.

**Consequences.**
- Failed and denied tool runs become visible in compact summaries even for trace-sparse providers.
- Cost: the legacy max overlay is conservative and may undercount the true union until both streams share an invocation id.

## ADR-019 — Task-review feedback scores separately from PR review comments

**Status:** Accepted · 2026-04 · [T20260428-17], [T20260430-4], [T20260430-5]

**Context.** Local Orbit task review threads and GitHub PR review comments are different workflow artifacts, and reply volume should not be scored as distinct review findings.

**Decision.** Keep `pr.review_comments` for synced PR/GitHub comments, score local review-thread creations separately as `task-review-threads` surfaced as `task_review.threads`, do not score replies, and accept only exact configured or built-in model identities.

**Consequences.**
- Local review feedback earns immediate task-review credit while synced PR feedback remains a separate PR metric.
- Cost: review productivity now has two counters, and aggregate views must label them clearly rather than adding them blindly.

## ADR-020 — Command-audit execution ids are process-disambiguated

**Status:** Accepted · 2026-05 · [T20260505-6]

**Context.** Timestamp-only command-audit execution ids collided when concurrent `orbit tool run orbit.task.show` processes in one workspace generated ids at the same effective clock tick.

**Decision.** Generate command-audit execution ids through one shared helper that combines a stable prefix, wall-clock nanoseconds, process id, and a per-process atomic sequence while keeping the SQLite unique index authoritative.

**Consequences.**
- Parallel CLI and runtime audit producers get deterministic collision resistance without weakening uniqueness constraints.
- Cost: execution ids are longer and less visually compact than the old `exec-<nanos>` shape.

## ADR-021 — Loop audit JSONL files materialize on first loop event

**Status:** Accepted · 2026-05 · [T20260506-2]

**Context.** V2 runs always constructed both the v2 envelope sink and the loop-level sink. Runs that emitted only envelope events or CLI-backend blobs therefore left zero-byte `.orbit/state/audit/loop/{run_id}.jsonl` files beside populated `v2_loop` files, making the audit tree look noisy and misleading.

**Decision.** Keep the loop sink available for HTTP agent-loop events and blob writes, but defer creating `loop/{run_id}.jsonl` until the first `LoopAuditEvent` is emitted. Blob writes continue to use `.orbit/state/audit/blobs/` without creating an empty loop event file.

**Consequences.**
- Runs with no loop-level provider/tool events no longer leave empty loop JSONL placeholders.
- Cost: consumers must treat a missing loop JSONL file as "no loop events were emitted", not as a missing run; the v2 envelope file remains the canonical run spine.

## ADR-0279 — Automated git commits carry implementer authorship

**Status:** Accepted · 2026-07 · [ORB-10458] · legacy_id: `auditability/ADR-022`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0279"}'`.

---

## ADR-0280 — Workflow git commit identity is process-scoped

**Status:** Accepted · 2026-07 · [ORB-10458] · legacy_id: `auditability/ADR-023`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0280"}'`.

---

## ADR-024 — Friction reports are append-only records, not lifecycle tasks

**Status:** Accepted · 2026-05 · [T20260510-13]

**Context.** Friction reports are operational signal, not planned work. Storing them as task records cluttered task lists and forced accept/reject triage decisions that were more about duplicate handling than report validity.

**Decision.** Store friction reports under `.orbit/frictions/{yyyy}-{mm}/F{nnn}.md` with YAML frontmatter and markdown body. Expose only `orbit.friction.*` artifact operations; exclude `friction` from the task status taxonomy and reject it during task parsing; compute rates on demand from friction records plus task completion attribution.

**Consequences.**
- The backlog contains work items rather than self-report signal, and friction reports remain append-only.
- The migration window is closed; task CLI, MCP, dashboard, and workflow surfaces no longer expose a friction status.
- Cost: workspaces with unmigrated legacy friction tasks must migrate them before upgrading because task deserialization no longer accepts `status: friction`.

---

## ADR-0164 — Ship PR transitions preserve task implementer attribution

**Status:** Accepted · 2026-05 · [ORB-00106]

**Context.** `orbit run ship` reached PR-open Review handoff and PR-merge Done handoff through system-owned automation even when the workflow had a resolved implementer identity. Prior attribution fixes in [ORB-00067], [ORB-00089], and [ORB-00091] covered adjacent automation paths, but the ship PR loops still had two real alternatives: trust the ship actor/runtime context, or carry task/run provenance explicitly.

**Decision.** Ship-path PR transitions carry attribution on each automation update. The Review handoff uses existing `task.implemented_by`, then the pipeline's resolved implementer identity, then task-authored fallback fields (`planned_by`, `created_by`), leaving the genuine actor-less fallback as `system`. The Done handoff preserves existing `implemented_by`, otherwise uses `created_by`, then `system`. Regression tests exercise PR-open review stamping and distinct Done identities in one batch so a batch-level actor cannot homogenize them.

**Consequences.**
- Shipped task records, ship scoreboards, and follow-on git author derivation can preserve the implementer family that actually produced each task before and after PR review.
- Actor-less automation still records `system` instead of panicking or fabricating a family label.
- Cost: the ship pipeline must explicitly bridge task/run provenance into automation update payloads, so future edits to PR-open or PR-merge loops need to preserve the regression tests rather than assuming runtime actor context is enough.

---

## ADR-0245 — Derive invocation cost at query time from a versioned price table

**Status:** Accepted · 2026-07 · [ORB-10338] [ORB-10370] [ORB-10579]

**Context.** The invocation store already retained exact per-invocation token splits, but had no notion of USD cost — cost existed only as a provider-reported total buried in the worker's unparsed per-run JSON, never joined to a model or token split. [ORB-10338] adds cost. Two real alternatives existed: (a) compute cost once at ingest time and store it as a frozen column, or (b) keep rows token-only and derive cost from a versioned price table looked up by exact model string and the invocation's timestamp on every read/aggregate.

**Decision.** Cost is derived at query time, not stored. `orbit_common::types::pricing` ships a versioned price table as an in-repo YAML asset (`crates/orbit-common/assets/model_prices.yaml`), keyed by exact model string plus an `effective_from`/`effective_until` date range and an input-token billing basis, parsed once behind a `OnceLock` cache. Exclusive input totals remain the backward-compatible default; gross totals include cache buckets, which derived pricing removes with checked arithmetic before charging the full input rate. `InvocationRecord` gains `derived_cost_usd` (computed at read time from the row's token splits, model, and timestamp against the price table) alongside a new `provider_cost_usd` column that persists the provider's own reported total verbatim for monthly manual reconciliation. Adding or correcting a price row is a YAML edit, not a database migration.

**Consequences.**
- Historical invocation rows re-price automatically when a price row is corrected or backfilled — no migration/backfill script needed to fix a wrong rate.
- `derived_cost_usd` is `None` whenever no price row covers a model/date, so unpriced or newly-launched models degrade to "unknown" rather than a silently wrong number.
- `provider_cost_usd` never changes once written, so it stays the ground truth Daniel reconciles against monthly even if `derived_cost_usd` for the same row changes later.
- [ORB-10370] wires Claude CLI `total_cost_usd` into that column from the same parse that captures provider model identity; providers that report no USD total keep `NULL`.
- Cost: because derived cost is recomputed on every read instead of frozen at ingest, editing a price row after the fact silently changes the reported cost of every past invocation under that model/date range — there is no record of what a row's derived cost "used to be", unlike the immutable `provider_cost_usd`.
- Cache-write TTL split: `TokenUsage`/`PriceRow` distinguish 5-minute-TTL cache-creation tokens (`cache_create`, 1.25x input) from 1-hour-TTL (`cache_create_1h`, 2x input), since Anthropic prices them differently; the store persists both. Validated against real worker run 91d7ef01 (`claude-opus-4-8[1m]` → $1.014018 exactly). Claude response parsing retains the provider's TTL split. Codex JSONL and OpenAI-compatible parsing retain standard writes when reported; OpenAI's 1-hour bucket stays zero, and an anomalous stored nonzero count is not priced as free.
- GPT-5.6 cost is a standard short-context API-equivalent estimate. Exact Fast/service-tier and long-context pricing is deliberately unknown until per-request service-tier and context dimensions are retained.
- Cost: gross-input rows with cache detail larger than the gross total now return unknown rather than producing a partial estimate, so malformed telemetry can reduce reported cost coverage.
- Model-string keying: rows are keyed by the exact string that lands in `InvocationRecord.model`. A context-window suffix (`claude-opus-4-8[1m]`) is stripped to fall back to the base row rather than duplicating rows, since it bills at base rates; a distinct `model[1m]` row would win by exact match if a long-context premium ever applied. After [ORB-10370], provider-reported CLI model keys replace configured aliases at ingest when available; the configured value remains the fallback, and a structured warning retains requested/reported disagreement without adding a second database column.

---

## ADR-0249 — Workflow commit authors use the persisted crew model

**Status:** Superseded by [ADR-0299] · 2026-07 · [ORB-10519]

**Context.** Pipeline-created commits exposed only a generic or family author even though the job run already persisted the exact resolved crew model used as `AGENT_MODEL` for provider subprocess commit trailers. Deriving attribution again from `task.implemented_by` or crew aliases, or letting the author and trailer read different process state, would permit the ambient author to disagree with durable model telemetry.

**Decision.** Read the persisted job-run `crew_model` once and use that same opaque string both to construct the author name `orbit (<model>)` and to set the spawned Git process's `AGENT_MODEL` for `prepare-commit-msg`. Use `agent@orbit.invalid`; do not resolve aliases, validate model strings, or add a model registry. A missing model uses the generic `orbit <orbit@orbit.local>` author. Keep the committer as the process-scoped generic Orbit identity, and adopt existing commits without amendment.

**Consequences.**
- `git log --format=%an` distinguishes pipeline commits produced by different resolved models, while the model-bearing author and `Agent-Model` trailer cannot diverge.
- Existing `Agent-Run`, `Agent-Task`, and `Co-Authored-By` trailers remain additive and unchanged.
- ORB-10365 retains a host committer because its already-created commit was adopted forward-only, while ORB-10348 was created by pipeline automation with a scoped Orbit committer.
- Cost: a bare `[crews.*].model` value remains bare in the author because configured model strings stay opaque and Orbit ships no release-coupled alias table.

---

## ADR-0299 — Workflow alone creates shipment commits while dirty failures remain recoverable

**Status:** Accepted · 2026-07 · [ORB-10519]

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0299"}'`.

---

## ADR-0297 — Provider subprocess liveness is a separate audit event probed at read time

**Status:** Accepted · 2026-07 · [ORB-10496]

**Context.** A ship-pipeline (`workflow_ship`) implementation agent is a CLI subprocess of the pipeline worker, not of the Worker daemon behind `agent_invoke`, so bridge `agent_run_list` never sees it and `child.id()` existed only inside `cli_runner/supervisor.rs` for process-group cleanup. During run-rescue (F2026-07-083 / [ORB-10257]) a healthy long-running agent was indistinguishable from a dead child without shell process-tree inspection. Two shapes were available: a periodic heartbeat written for as long as the child lives, or a single spawn-time PID record whose liveness the reader probes. Extending `cli.invocation.started` was not available — it is emitted before spawn, so no PID exists yet.

**Decision.** Emit one `cli.invocation.process` event (`provider`, `pid`, `pid_start_time`) immediately after spawn, ordered strictly between `cli.invocation.started` and `cli.invocation.finished`; the envelope writer persists synchronously, so it is readable mid-invocation. Liveness is derived at read time by `orbit_common::utility::process_identity::probe_process_liveness` (`kill(pid, 0)` plus the Linux zombie check, with the recorded start-identity token rejecting a recycled PID). `OrbitRuntime::collect_run_provider_processes` pairs each process event with the exit event that closes it within the same step and probes only the still-open ones; `GET /api/runs/:id` (bridge `workflow_run_status`) and `orbit run show` project the result.

**Consequences.**
- A long-running `agent_implement` step is distinguishable from a lost child without shell access, which is the decision run-rescue actually has to make.
- Retries within one step pair in order (newest still-open record wins), so each attempt reports separately instead of collapsing onto the first spawn.
- A live PID whose versioned start-identity token disagrees reads as `exited`, so PID reuse cannot fake a live agent.
- An unprobeable host degrades to `alive`/`unknown` rather than `exited`, matching the existing job-run owner-reconciliation policy that a probe which cannot answer is never proof of death.
- Cost: liveness is only as fresh as the query and only meaningful on the host that ran the child — a remote or later reader sees `exited` for every historical open invocation, because the answer comes from the local process table rather than from the persisted event. A heartbeat would have survived that, at the price of a write per interval per invocation and a staleness threshold to tune.
- Cost: `pid_start_time` costs one `ps` per provider spawn; a sandbox that blocks `ps` yields `None`, weakening the record to unguarded-PID liveness rather than failing the spawn.

---

## ADR-0323 — Friction records carry an author-settable title; derivation is a structural fallback

**Status:** Proposed · 2026-08 · [ORB-10590]

**Context.** A friction record's handle was not a field. `title` existed only on the wire, derived by the read projection from the body's first non-empty line. No write surface accepted one, and nothing said the first line was load-bearing. On a 41-record corpus the derivation tracked authoring style rather than content: a body opening with a section heading derived that label as its title, a body opening with a long lead paragraph derived the whole paragraph. Two records six days apart documented the same bug under the same generic label, so the pre-filing search for prior art found nothing recognisable.

**Decision.** `title` becomes a stored `Option<String>` on the record and its frontmatter, settable through `orbit.friction.add` and retitleable through `orbit.friction.update` without touching the append-only body. Derivation moves to write time and persists what it produced; it stays as the read-side fallback so records written before the field existed need no migration. Derivation is structural, never lexical: a leading heading is a title only when no sibling heading at its level or shallower follows it, a leading bold run is an inline lead-in whose sentence is the subject, and the result is clamped to `FRICTION_TITLE_MAX_CHARS`. No `summary` field is introduced — the record keeps one short handle plus the full report.

**Consequences.**
- Every record lands with a handle naming its subject, and the existing corpus self-heals on read for both failure modes.
- A hardcoded list of generic section headings was rejected: it encodes one language and one house style, needs an edit per new label, and cannot address the overlong case at all. Heading count alone separated every well-titled record from every badly-titled one in the surveyed corpus.
- Refusing an add whose derived title looks non-identifying was rejected as a break for existing callers and the same brittle judgement in a different place.
- Cost: the `orbit.friction.add` / `.update` MCP schemas gain a parameter. Additive and optional, but still release-visible schema drift.
- Cost: the structural rules can still under-serve a body whose subject appears in its second sentence; the stored field is the escape hatch, which is why derivation is only the fallback.
- Cost: `--title` is unreachable until the deployed `orbit` binary is rebuilt, so records that need a human title cannot be retitled the moment this lands. The data pass for the records that motivated the decision is [ORB-10598].

Full narrative: `orbit tool run orbit.adr.show --input '{"id":"ADR-0323"}'`.

---

## ADR-0345 — Friction records move to SQLite with a legacy-evidence `path` projection

**Status:** Proposed · 2026-08 · [ORB-10680]

**Context.** Friction list, filtered-query, and stats operations discovered every Markdown record under a workspace's friction tree, parsed every YAML envelope and body, allocated the complete corpus as `Vec<StoredFrictionRecord>`, and only then filtered, sorted, paginated, or aggregated. Peak memory and parse work therefore grew with total retained friction history even when a caller asked for a 50-row page or a narrow status filter. The file-backed rationale no longer matched the runtime contract: frictions are hub-only coordination state, writes go through Orbit surfaces rather than human file edits, and the canonical hub already copies checkout-local records into a global per-workspace file tree. A public `path` field pointed at the backing Markdown file, so moving persistence could not silently leave it fictitious.

**Decision.** Friction records move into the host-global store under schema migration v12 `friction_records_sqlite`, keyed by the composite `(workspace_id, friction_id)` so IDs stay workspace-local and identical IDs in two workspaces coexist ([L-0072]). Every list predicate — workspace, status, model, tag, date range, free text — plus the ordering, the page, and every stats aggregate is pushed into SQL, so a bounded request decodes at most the rows it asked for and `stats` decodes none. Monthly ID allocation runs inside the same write transaction as the insert, backed by a unique `(workspace_id, month, seq)` index. Each workspace's legacy tree is imported exactly once, transactionally and idempotently: a malformed record, a friction ID claimed twice in one source tree, an ID that does not address the file holding it, or a discovered/handled count mismatch aborts the transaction, and an interrupted import commits nothing. After the marker commits, SQLite is the sole live read and write source. The public `path` field is retained and now reports the legacy evidence file an imported record came from, and `null` for any record written after cutover, rather than a fabricated location; the CLI renders it as `Legacy file`. The tag taxonomy stays a small YAML configuration file — moving record persistence does not move configuration.

**Consequences.**
- Bounded scan memory and indexed workspace-local reads for the CLI, MCP, HTTP, dashboard, Bridge-facing, and scoreboard friction paths; the scoreboard consumes a per-model SQL aggregate (`ScoreboardInputs::friction_reported`) instead of the full record slice.
- Legacy trees stay untouched, read-only rollback evidence for one release, and `export_workspace_frictions` re-materializes the live corpus in the same Markdown layout for inspection.
- No retention, deletion, cold archival, body compression, or disk-reclamation policy is introduced; removing legacy trees needs an explicit later finalize path that also drops `legacy_path`.
- Free-text `q` matching moves from Rust's Unicode-aware lowercasing to SQLite's ASCII `lower()`, so a non-ASCII uppercase query term matches case-sensitively where it previously did not.
- Cost: a consumer that read `path` as an always-present file location now sees `null` for post-cutover records and must treat it as an optional legacy pointer.
- Rejected alternative: keeping the file store and adding a SQLite index sidecar. That would have preserved two sources of truth for the same records and left the full parse cost on every cold read and index rebuild.

Full narrative: `orbit tool run orbit.adr.show --input '{"id":"ADR-0345"}'`.

---

## Task References

- **[T20260419-0002]** — Add workspace provenance and v2 audit envelope events for activity/job execution.
- **[T20260426-0519]** — Move file-backed activity/job audit traces under workspace state.
- **[T20260426-0526]** — Persist v2 invocation traces for metrics beside audit.
- **[ORB-00190]** — Retire the metrics CLI and make dashboard endpoints canonical for invocation metrics.
- **[T20260426-0605]** — Add this auditability design folder and record initial ADRs.
- **[T20260426-0705]** — Expose v2 run audit events through `orbit run events` and `orbit run trace`.
- **[T20260426-0709]** — Align run step selectors on activity `step.id` and move CLI invocation log reading behind orbit-core runtime accessors.
- **[T20260426-2313]** — Stream CLI subprocess stdout/stderr through structured tracing events.
- **[T20260426-2343]** — Add the global process tracing JSONL feed at `~/.orbit/state/logs/orbit.jsonl`.
- **[T20260426-2349]** — Apply tracing-layer redaction before stderr and global JSONL output.
- **[T20260427-0023]** — Project policy denials and friction task submissions into the global tracing feed.
- **[T20260427-27]** — Close out the unified-log story: job lifecycle dual-write, library print migration with workspace lint gate, and `orbit log tail` reader CLI.
- **[T20260427-43]** — Add `status: friction`, creation-time friction routing, migration, and history-derived friction bounty refresh.
- **[T20260427-44]** — Add shared log formatter extraction and dashboard backend `/api/log` snapshot/SSE endpoints.
- **[T20260427-46]** — Implement the Gemini-owned Tasks-tab `orbit.log` panel using the shared dashboard backend API.
- **[T20260427-47]** — Allow explicit task attribution correction for `planned_by` and `implemented_by` through task update paths.
- **[T20260427-52]** — Deprecate `agent` in normal tool-call JSON, infer agent family from `model`, and reject inconsistent legacy pairs.
- **[T20260428-4]** — Record audit events for MCP tool invocations by moving ownership into the runtime, adding the entry-point discriminator, and bracketing MCP preflight.
- **[T20260428-7]** — Correlate command-audit rows with originating run/task/activity by adding nullable correlation columns and surfacing them on the dashboard.
- **[ORB-10228]** — Supersede ADR-017 caller-JSON precedence for MCP; add trusted caller/process provenance, capability sets, and call/lease correlation.
- **[T20260428-11]** — Derive compact scoreboard all/failed tool-call counts from command-audit tool-run rows.
- **[T20260428-17]** — Split local Orbit task-review scoring from PR review-comment scoring and surface both in compact scoreboards.
- **[T20260430-4]** — Count local task-review score by review-thread creations, not replies, and rename the task-review summary field to `threads`.
- **[T20260430-5]** — Tighten task and PR review-message scoring so only exact configured orchestrator/helper model identities score; typo-prefixed labels are ignored.
- **[T20260430-20]** — Shorten the auditability docs while preserving required guarantees.
- **[T20260505-6]** — Replace timestamp-only command-audit execution ids with process-disambiguated generated ids for parallel tool runs.
- **[T20260506-2]** — Lazily materialize loop audit JSONL files only when loop-level events are emitted.
- **[T20260508-22]** — Use `task.implemented_by` to set git commit authors for automated task commits.
- **[T20260509-12]** — Scope workflow git author and committer identity to the spawned commit process without writing repo-local Git config.
- **[T20260510-13]** — Move friction reports from task lifecycle state to append-only `.orbit/frictions/` records.
- **[ORB-00067]** — Earlier automation attribution work that did not close the ship batch PR Done transition gap.
- **[ORB-00089]** — Earlier system-attribution gap that informed the ship-path fallback rule.
- **[ORB-00091]** — Prior fix for automation-driven status attribution that did not cover the ship merge loop.
- **[ORB-00080]** — Collapse Orbit agent identity to family and isolate exact model strings to invocation/configuration surfaces.
- **[ORB-00090]** — Align agent-facing docs and tool descriptions with the family-as-identity convention.
- **[ORB-00106]** — Preserve per-task implementer attribution when `orbit run ship` moves batch PR tasks from Review to Done.
- **[ORB-10202]** — Remove the retired friction task status and consolidate task mutation attribution and record-parameter construction.
- **[ORB-10338]** — Add the versioned model price table and query-time `derived_cost_usd`, plus a persisted `provider_cost_usd` column for reconciliation.
- **[ORB-10370]** — Fill provider model/cost trace fields from CLI result JSON and prefer reported model identity at invocation ingest.
- **[ORB-10579]** — Correct GPT-5.6 price periods, cache-write rates, gross-input accounting, and standard short-context estimate boundaries.
- **[ORB-10519]** — Keep the persisted crew-model author and process-scoped Orbit committer while removing hook-specific trailer input and provider-commit adoption ([ADR-0299], superseding [ADR-0249] and [ADR-0294]).
- **[ORB-10369]** — Introduce the persisted resolved crew model as the pipeline commit author with generic fallback and no alias resolver ([ADR-0249], superseded by [ADR-0299]).
- **[ORB-10496]** — Record the spawned provider subprocess PID as its own audit event and expose read-time liveness through run status and `orbit run show`.

- **[ORB-10590]** — Make the friction record handle an author-settable field and derive it structurally when omitted ([ADR-0323]).
- **[ORB-10680]** — Moved hub friction records into the host-global SQLite store to bound scan memory ([ADR-0345]).

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
