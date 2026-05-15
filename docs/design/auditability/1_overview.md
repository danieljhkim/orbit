# Auditability — Overview

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-15

Auditability is Orbit's answer to the operator question that matters after an agent touches a real repository: what happened, why, and who is accountable? The contract spans command rows, Orbit tool mutations, activity/job runs, provider turns, tool calls, filesystem denials, task attribution, metrics, and redacted payload storage. [2_design.md](./2_design.md) describes the shipped implementation; [3_vision.md](./3_vision.md) names the remaining gaps.

---

## 1. Motivation

Orbit runs fleets of agents against user-owned repositories, so auditability is a product feature rather than an observability afterthought:

1. **Replayable accountability.** README and positioning docs promise enough context for every meaningful action to answer what, why, and who. The dedicated design folder from [T20260426-0605] keeps that promise reviewable.
2. **Layered evidence.** A command row says which command ran. A v2 envelope says which workflow step ran. Loop events and blobs preserve provider/tool detail. The layers must remain related without becoming one oversized record.
3. **Durable identity.** Task author fields, command audit roles, v2 `agent_identity`, invocation metrics, git commit identities, and commit/task metadata all need to point back to a concrete actor or model.
4. **Write-side secrecy.** Orbit preserves useful payloads while redacting provider keys and sensitive environment-derived values before durable storage.
5. **Explicit gaps.** Silent mutation paths are bugs. Missing coverage belongs in the coverage matrix, not in tribal memory.

---

## 2. Core Concepts

### 2.1 Command audit records are queryable SQLite rows

The CLI audit middleware and runtime tool-dispatch paths write persistent `AuditEvent` records for most top-level commands and tool calls. These compact rows back `orbit audit list`, `orbit audit show`, `orbit audit stats`, and export commands. They carry command/target metadata, actor role, status, timing, working directory, host, process id, and optional argument/error fields, but not full provider transcripts.

### 2.2 Activity/job run traces are file-backed JSONL trees

The v2 activity/job runtime emits `V2AuditEvent` envelopes for run, step, activity, fan-out, loop, filesystem, denial, and CLI-backend lifecycle events under `.orbit/state/audit/v2_loop/`. This layer is the workflow replay spine: it carries `run_id`, `event_id`, `parent_event_id`, `agent_identity`, and optional `workspace_path`. `orbit run events`, `orbit run trace`, `orbit run show -s`, and `orbit run logs -s` expose the same activity DAG `step.id` source of truth after [T20260426-0705] and [T20260426-0709].

### 2.3 Agent-loop audit events preserve provider and tool detail

The HTTP loop engine emits `LoopAuditEvent` records for sessions, HTTP requests/responses, tool requests/results, iteration boundaries, and policy denials. Loop JSONL materializes under `.orbit/state/audit/loop/` only once a run emits loop-level events; large request, response, input, and output bodies are stored as redacted content-addressed blobs under `.orbit/state/audit/blobs/`.

### 2.4 Invocation metrics are adjacent, not a replacement

The invocation store records token usage, tool-call counts, task IDs, agent, model, job run, and activity IDs for metrics and scoreboards. It helps answer cost and usage questions, but it summarizes rather than preserves transcript structure. V2 job metrics began persisting beside audit in [T20260426-0526].

### 2.5 Redaction happens before durable payload storage

Blob writes apply pattern-based redaction at write time, and CLI error audit paths scrub sensitive live environment values before persistence. Readers should not need to re-scrub normal audit artifacts. The detailed contract lives in [specs/redaction-retention.md](./specs/redaction-retention.md).

### 2.6 Process tracing has a global JSONL feed

The default tracing subscriber appends redacted structured events to `~/.orbit/state/logs/orbit.jsonl` after [T20260426-2343] and [T20260426-2349]. The feed is global because logging initializes before workspace resolution. After [T20260427-0023], filesystem policy denials, proc-spawn allowlist denials, and friction task submissions also project stable `tracing::warn!` events beside their canonical stores.

---

## 3. At a Glance

| Concern | Where it lives | Primary task ID |
|---------|----------------|-----------------|
| Audit design ownership | `docs/design/auditability/` | [T20260426-0605] |
| Command audit records and queries | `crates/orbit-common/src/types/audit_event.rs`, `crates/orbit-cli/src/command/observe/audit.rs`, `crates/orbit-store/src/sqlite/audit_event_store.rs` | [T20260426-0605] |
| V2 activity/job envelopes and JSONL sink | `crates/orbit-common/src/types/activity_job/audit_envelope.rs`, `crates/orbit-engine/src/activity_job/audit_writer.rs` | [T20260419-0002], [T20260426-0519] |
| Run trace inspection CLI | `crates/orbit-cli/src/command/run/mod.rs`, `crates/orbit-core/src/runtime/run_audit.rs` | [T20260426-0705], [T20260426-0709] |
| Loop audit events and blobs | `crates/orbit-agent/src/loop_engine/audit/mod.rs`, `crates/orbit-common/src/utility/blob_store.rs` | [T20260426-0605] |
| Redaction utilities | `crates/orbit-common/src/utility/redaction.rs` | [T20260426-0605], [T20260426-2349] |
| Global tracing JSONL feed and live projections | `crates/orbit-common/src/utility/logging.rs`, selected FS/proc/task producers | [T20260426-2343], [T20260427-0023] |
| V2 invocation metrics persistence | `crates/orbit-store/src/sqlite/invocation_store.rs`, `crates/orbit-core/src/runtime/v2_host/mod.rs` | [T20260426-0526] |
| Task attribution fields | `crates/orbit-common/src/types/task.rs`, task update/runtime host paths | [T20260426-0605], [T20260427-47] |
| Workflow git commit identity attribution | `crates/orbit-engine/src/executor/automation/vcs/commit/` | [T20260508-22], [T20260509-12] |

---

## Task References

- **[T20260419-0002]** — Add workspace provenance and v2 audit envelope events for activity/job execution.
- **[T20260426-0519]** — Move file-backed activity/job audit traces under workspace state.
- **[T20260426-0526]** — Persist v2 invocation traces for metrics beside audit.
- **[T20260426-0605]** — Add this auditability design folder and establish codex ownership.
- **[T20260426-0705]** — Expose v2 run audit events through `orbit run events` and `orbit run trace`.
- **[T20260426-0709]** — Align run step selectors on activity `step.id` and move CLI invocation log reading behind orbit-core runtime accessors.
- **[T20260426-2343]** — Add the global process tracing JSONL feed at `~/.orbit/state/logs/orbit.jsonl`.
- **[T20260426-2349]** — Apply tracing-layer redaction before stderr and global JSONL output.
- **[T20260427-0023]** — Project policy denials and friction task submissions into the global tracing feed.
- **[T20260427-47]** — Allow explicit task attribution correction for `planned_by` and `implemented_by` through task update paths.
- **[T20260430-20]** — Shorten the auditability docs while preserving required guarantees.
- **[T20260506-2]** — Lazily materialize loop audit JSONL files only when loop-level events are emitted.
- **[T20260508-22]** — Use `task.implemented_by` to set git commit authors for automated task commits.
- **[T20260509-12]** — Scope workflow git author and committer identity to the spawned commit process without writing repo-local Git config.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
