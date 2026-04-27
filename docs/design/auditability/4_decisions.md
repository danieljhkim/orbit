# Auditability — Decisions

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-04-27 (T20260427-44, T20260427-46)

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

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
