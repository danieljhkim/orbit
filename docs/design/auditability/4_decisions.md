# Auditability — Decisions

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-17 (ORB-00106)

This is the append-only ADR log for Auditability. Entries are ordered by ADR number. New entries should use the template in [../CONVENTIONS.md](../CONVENTIONS.md) and cite the task that made the decision real.

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
- `orbit metrics` and scoreboards can avoid parsing audit JSONL.
- Cost: metrics can diverge from transcript detail if a provider path reports incomplete usage.

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

## ADR-012 — Friction scorekeeping derives from lifecycle history

**Status:** Superseded · 2026-05 · [T20260510-13]

**Context.** Friction reports once used a dedicated task type, but untriaged reports shared `status: proposed` with human-authored proposals, making scoreboard derivation ambiguous.

**Decision.** Add `status: friction` as the creation status for self-reports, infer legacy friction routing at creation, and rebuild `friction_bounty.json` from task history.

**Consequences.**
- Friction inbox items are separated from human proposals while legacy friction task records remain readable.
- Cost: legacy untriaged reports need migration, and already-triaged legacy histories depend on existing transition records.

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

**Status:** Accepted · 2026-04 · [T20260428-7]

**Context.** SQLite command-audit rows recorded tool invocations but had no direct link to the task, job run, activity, or step that caused them.

**Decision.** Add nullable `task_id`, `job_run_id`, `activity_id`, and `step_index` columns, populate them at runtime tool dispatch from caller JSON first and engine env vars second, index task/run ids, and render the fields in dashboard detail rows.

**Consequences.**
- Operators can drill from a tool row to the originating task and run context without out-of-band correlation.
- Cost: historical rows remain NULL, and caller-asserted JSON values are weaker evidence than engine-supplied env context.

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

## ADR-022 — Automated git commits carry implementer authorship

**Status:** Accepted · 2026-05 · [T20260508-22]

**Context.** Task records already store `implemented_by`, but automated `git_commit` actions previously delegated commit authorship to local git config, hiding the agent that actually produced the change.

**Decision.** Pass a per-commit `--author` derived from `task.implemented_by` for single-implementer commits. Mixed-implementer batch commits use `orbit <orbit@orbit.local>` as the aggregate author and add one `Co-Authored-By` trailer per distinct implementer identity. ADR-023 extends this provenance to committer identity without reusing repo-local user config.

**Consequences.**
- Reviewers can see implementation provenance directly in git history without joining back through run audit events.
- Local git config is not written by workflow commit automation and is no longer the source of committer identity for those commits.
- Cost: multi-implementer batch commits require trailer-aware attribution queries; `git log --author` finds the aggregate commit author, not every co-author trailer.

---

## ADR-023 — Workflow git commit identity is process-scoped

**Status:** Accepted · 2026-05 · [T20260509-12]

**Context.** Reusing local Git config for workflow committers made agent identities sticky in developer repositories. If `user.name` or `user.email` was set to an agent identity in repo-local config, later human commits inherited that attribution.

**Decision.** Automated `git_commit` actions set author and committer identity only for the spawned `git commit` process. Single-implementer commits use that implementer's scoped identity for both author and committer. Mixed-implementer commits use `orbit <orbit@orbit.local>` as the aggregate author and committer while preserving distinct implementers as `Co-Authored-By` trailers. Workflows must not write agent or aggregate identities into repo-local Git config.

**Consequences.**
- Human `user.name` and `user.email` settings remain byte-for-byte stable across workflow commits.
- Worktrees with no local `user.*` config can still create workflow-owned commits with explicit provenance.
- The public `git.commit` tool remains user-directed and ambient-config based; workflow-owned commit automation uses this scoped path instead.

---

## ADR-024 — Friction reports are append-only records, not lifecycle tasks

**Status:** Accepted · 2026-05 · [T20260510-13]

**Context.** Friction reports are operational signal, not planned work. Storing them as task records cluttered task lists and forced accept/reject triage decisions that were more about duplicate handling than report validity.

**Decision.** Store friction reports under `.orbit/frictions/{yyyy}-{mm}/F{nnn}.md` with YAML frontmatter and markdown body. Expose only `orbit.friction.add/list/show/stats`; reject new `orbit.task.add` calls that request legacy friction task routing or `status: friction`; compute rates on demand from friction records plus task completion attribution.

**Consequences.**
- The backlog contains work items rather than self-report signal, and friction reports remain append-only.
- Cost: legacy friction tasks remain readable artifacts and need a one-shot migration command to copy them into the new corpus.

---

## ADR-0164 — Ship Done transitions preserve task implementer attribution

**Status:** Accepted · 2026-05 · [ORB-00106]

**Context.** `orbit run ship` reached the Review -> Done transition through system-owned automation even when each task already carried agent provenance. Prior attribution fixes in [ORB-00067], [ORB-00089], and [ORB-00091] covered adjacent automation paths, but the batch PR merge loop still had two real alternatives: trust the ship actor/runtime context, or carry each task provenance explicitly.

**Decision.** Ship-path Done transitions use per-task provenance as the source of truth: `task.implemented_by` wins, then `task.created_by`, then the genuine actor-less fallback remains `system`. The merge loop passes that resolved value on the task update for each task, and the regression test exercises distinct identities in one batch so a batch-level author cannot homogenize them.

**Consequences.**
- Shipped task records, ship scoreboards, and follow-on git author derivation can preserve the implementer family that actually produced each task.
- Actor-less automation still records `system` instead of panicking or fabricating a family label.
- Cost: the ship pipeline must explicitly bridge task provenance into the automation update payload, so future edits to that loop need to preserve the regression test rather than assuming runtime actor context is enough.

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
- **[T20260427-43]** — Add `status: friction`, creation-time friction routing, migration, and history-derived friction bounty refresh.
- **[T20260427-44]** — Add shared log formatter extraction and dashboard backend `/api/log` snapshot/SSE endpoints.
- **[T20260427-46]** — Implement the Gemini-owned Tasks-tab `orbit.log` panel using the shared dashboard backend API.
- **[T20260427-47]** — Allow explicit task attribution correction for `planned_by` and `implemented_by` through task update paths.
- **[T20260427-52]** — Deprecate `agent` in normal tool-call JSON, infer agent family from `model`, and reject inconsistent legacy pairs.
- **[T20260428-4]** — Record audit events for MCP tool invocations by moving ownership into the runtime, adding the entry-point discriminator, and bracketing MCP preflight.
- **[T20260428-7]** — Correlate command-audit rows with originating run/task/activity by adding nullable correlation columns and surfacing them on the dashboard.
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

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
