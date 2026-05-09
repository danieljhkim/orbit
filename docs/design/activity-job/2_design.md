# Activity / Job — Design

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-09 (T20260427-34, T20260427-36, T20260427-38, T20260427-40, T20260508-3, T20260508-8, T20260509-2)

This document describes the shipped Activity / Job substrate across `orbit-common`, `orbit-engine`, `orbit-core`, and `orbit-cli`: asset shape, normalization, dispatch boundaries, backend semantics, DAG execution, audit, and retained legacy edges. See [1_overview.md](./1_overview.md) for purpose and [3_vision.md](./3_vision.md) for open questions.

---

## 1. Asset Shape and Two-Pass Loading

Activity / Job assets are `schemaVersion: 2` YAML envelopes with:

- `kind: Activity` or `kind: Job`
- `metadata.name`
- typed `spec`

The loader in `crates/orbit-common/src/types/activity_job/asset_loader.rs` reads the schema header first, then parses the full envelope into `ActivityV2` or `JobV2`; that shape arrived in [T20260418-2010]. `schemaVersion: 1` is retired after [T20260419-2156], and `kind` mismatches are structural errors, so an activity cannot dispatch as a job or vice versa.

---

## 2. Activity Surface

`ActivityV2` carries shared fields:

- `description`
- `input_schema_json`
- `output_schema_json`
- optional `fsProfile`

and then flattens one `ActivityV2Spec` variant:

- `AgentLoop(AgentLoopSpec)`
- `Groundhog(GroundhogSpec)`
- `Deterministic(DeterministicSpec)`
- `Shell(ShellSpec)`

The common `agent_loop` fields are:

- `instruction`
- `tools`
- `on_denial`
- optional `model`
- `max_iterations`
- `backend`
- `provider`
- `wall_clock_timeout_seconds`

Groundhog has its own `GroundhogSpec`, but `as_agent_loop_spec()` projects it into an HTTP-backed agent loop when the runner needs the shared transport path. That sibling kind landed in [T20260420-0510-2].

`DeterministicSpec` is just `{ action, config }`. `ShellSpec` is a direct subprocess surface with `program`, `args`, `allowed_programs`, `timeout_seconds`, and `expected_exit_codes`.

---

## 3. Job Surface

`JobV2` carries:

- `state`
- optional `default_input`
- `max_active_runs`
- `kind`
- `steps`

`JobKind` is currently `workflow` or `subroutine`, added in [T20260419-0339]. The more interesting surface is the step grammar from [T20260418-2018]:

- every step has `id`
- every step may add `when`
- every step may add `retry`
- every step chooses exactly one body

The body is one of:

- flat `TargetStep`
- named `TargetRef`
- `parallel`
- `fan_out` plus matching `fan_in`
- `loop`

`TargetStep` is the executor-facing form. It inlines an `ActivityV2Spec` plus optional `fsProfile`, `default_input`, `timeout_seconds`, and optional `session`. `TargetRef` is the authoring-facing form: `target: activity:<name>`. It is resolved away before execution.

Step-local input layering landed earlier, but the shipped job-level `default_input` behavior changed in [T20260423-0445]:

- if the caller passes `null`, the run input becomes `job.default_input`
- if both the caller input and `job.default_input` are JSON objects, Orbit performs a shallow merge and caller-supplied keys win on conflict
- if the caller input is any non-object JSON value, it replaces `job.default_input` entirely

Step-level `default_input` is still recursively template-rendered before dispatch. Support landed in [T20260413-0141], entered the v2 DAG path in [T20260418-2018], and was corrected for job-level merges in [T20260423-0445].

---

## 4. Load-Time Normalization Pipeline

orbit-core normalizes raw YAML before dispatch.

Catalog-discovered v2 jobs use `MergeByKey` precedence after [T20260425-0204]: `ORBIT_JOB_DIR` / `ORBIT_V2_JOB_DIR` entries first, then workspace jobs, then global seeded jobs. The first valid `metadata.name` wins, so a workspace `task_auto_pipeline` overrides the global default without making `orbit run ship` fail. Duplicate names inside one directory tree remain invalid because that single layer would otherwise be ambiguous.

Activity catalogs follow the same first-wins rule after [T20260426-0047]: `ORBIT_ACTIVITY_DIR` / `ORBIT_V2_CATALOG_DIR` entries first, then workspace activities, then global seeded activities. This lets a workspace carry an override such as `pr_open` without `orbit activity list --ops` failing on the duplicate global default. Duplicate names inside one activity directory tree remain invalid.

Direct single-activity runtime helpers:

1. Read YAML from disk.
2. Parse via `load_activity_asset(...)`.
3. Resolve `backend: auto` to a concrete backend.
4. Build audit sinks and run id with `system` as the v2 envelope `agent_identity`.
5. Dispatch the concrete `ActivityV2Spec`.

Job runs:

1. Read YAML from disk.
2. Parse via `load_job_asset(...)`.
3. Build the activity catalog from seeded/workspace activity directories.
4. Resolve every `target: activity:<name>` into a concrete `TargetStep` and resolve any job-level or step-level `recovery_activity` into a cached activity spec.
5. Resolve every `backend: auto` in the now-concrete step tree.
6. Reject loop-body `session:` bindings that resolve to `backend: cli`.
7. Build audit sinks and run id with `system` as the v2 envelope `agent_identity`.
8. Execute the normalized `JobV2`.

The target-ref pass was added in [T20260418-2019], backend resolution and `run-v2` entrypoints in [T20260418-2143], and CLI backend plus HTTP-only loop/session rejection in [T20260419-0104].

The public CLI now executes activity assets through jobs rather than exposing a standalone `orbit activity run` subcommand. `orbit activity` is an inspection/catalog surface; `orbit job run` and workflow aliases under `orbit run` are the public execution surfaces after [T20260426-0047].

Some module comments still describe older phase ordering; the authoritative behavior is the orbit-core call path in `crates/orbit-core/src/command/job_v2.rs`.

Seeded direct shipment workflows (`task_local_pipeline` and `task_pr_pipeline`) opt into `recovery_activity: step_failure_recovery` on specific steps after [T20260430-14]. The CLI-backed recovery agent receives only the executor-provided recovery keys, inspects the failed step, makes bounded repairs when safe, and returns before the executor's single post-recovery attempt. Higher-level orchestration workflows do not enable the hook because replaying child-run dispatch or planning orchestration is not a safe default recovery action.

The seeded `list_backlog_tasks` deterministic activity starts `task_auto_pipeline`. Automatic mode admits tasks by `status: backlog`, including accepted friction reports whose `type` remains `friction`, while untriaged `status: friction` reports stay out. It emits `task_count`, `task_ids`, `tasks`, singleton `bundles`, and an `excluded` array for admitted backlog tasks filtered because their context files overlap `in-progress` or `review` locks. `excluded` covers only lock overlap; status-based admission and `max_tasks` truncation stay silent, and explicit `task_ids` mode omits it. This attribution contract was added in [T20260421-0542-2] and the friction admission rule was updated in [T20260505-2].

`task_gate_pipeline` reserves a bundle's context files before it dispatches `task_pr_pipeline` or `task_local_pipeline` through `invoke_and_wait`. The reservation owner is the gate run that executed `reserve_locks`, not the child shipment run. Seeded defaults keep `ttl_seconds` aligned with `dispatch_timeout_seconds` at 7200 seconds, so the admission reservation covers the full child wait budget; workspace overrides must preserve `ttl_seconds >= dispatch_timeout_seconds` [T20260427-36]. Owned reservations are engine-cleaned when that owner run reaches a terminal state (`success`, `failed`, `cancelled`, or `timeout`), so correctness does not depend on every workspace override preserving a YAML release step. The seeded deterministic `release_locks` activity still calls `orbit.task.locks.release` after a terminal child wait as an early-release optimization; idempotent terminal cleanup then finds nothing left to release. After [T20260427-34], `invoke_and_wait` remains a raw child-status join primitive, and seeded shipment parents use `pipeline_success_guard` to fail after required cleanup whenever a child run reports anything other than `succeeded`. `task_gate_pipeline` guards the direct child after release; `task_auto_pipeline` guards collected gate results after fan-in and skips that guard for an empty backlog. Unowned/manual reservations remain explicit-release-or-TTL only. TTL is the fallback for abandoned/manual reservations or cases where no terminal cleanup or reserve-pressure reconciliation trigger runs. This lifecycle was tightened in [T20260430-26] and made engine-owned in [T20260505-10].

`task_epic_pipeline` loops over deterministic `load_epic` snapshots rather than trusting the orchestrator's prose response. The orchestrator's structured output is limited to the child gate run IDs it just dispatched; after [T20260427-40], a deterministic `pipeline_wait` step joins those runs before `refresh_epic` reloads task state, so the premium HTTP agent is not held open for child shipment. After [T20260427-38], `load_epic` treats `review` as a shipped stop state for epic automation: normal PR/local child workflows stop at that human handoff, so review subtasks are omitted from the next orchestrator input and can satisfy `all_terminal`. The final snapshot still preserves raw `status: "review"` while mapping the epic state to `done` for summary counters; this is pipeline completion, not human approval of the task lifecycle.

Reserve conflict checking also performs bounded, opportunistic stale-owned-reservation reconciliation before reporting reservation conflicts. It inspects only overlapping owned reservations under current reserve pressure; it is not a background sweeper. Existing job-run list/show reconciliation remains in place, and both paths release run-owned reservations with `release_reason: stale_run_reconciled` when they prove the owner is already terminal or stale. Release audit rows use the task-lock audit surface and include `reservation_id`, `owner_run_id` when present, and `release_reason` (`explicit`, `run_terminal`, `stale_run_reconciled`, or TTL expiration).

---

## 5. Backend Resolution and Constraint Rules

`Backend::Auto` is never supposed to reach dispatch. orbit-core resolves it once per run using the precedence chain implemented in `backend_resolver.rs`:

1. `--backend=<value>`
2. `ORBIT_BACKEND`
3. `[runtime] backend = "<value>"` in config
4. hard-coded fallback `http`

If any intermediate tier says `auto`, the resolver folds it to the hard-coded fallback so dispatch only sees `http` or `cli`. That rule arrived with `run-v2` in [T20260418-2143] and was hardened for CLI in [T20260419-0104].

The second rule is the HTTP-only feature constraint. Today that means loop-body cross-iteration `session:` binding: `validate_job_loop_session_backends(...)` rejects a `loop:` step with `session:` when it resolves to `backend: cli`.

The third rule is no silent provider fallback. `backend: http` against an unwired provider fails as `UnwiredHttpTransport` rather than launching a CLI runtime; providers and backends are separate schema choices.

The prescriptive contract for this area lives in [specs/backend-resolution.md](./specs/backend-resolution.md).

---

## 6. Engine-Core Boundary

Activity / Job is where orbit-core hands work to orbit-engine without depending on `orbit-agent` types.

`V2RuntimeHost` is the key boundary. orbit-core implements it and supplies five services back into the engine:

- run a deterministic action by name
- source an API key for a provider
- resolve a provider's CLI executor command plus static args
- build `ToolContext` for an activity, including policy, filesystem audit hooks, and trusted reservation-owner context from the active run id
- persist invocation traces for completed agent-loop work

That host wiring arrived in [T20260418-2143]. The cleanup in [T20260418-2210] kept the boundary primitive: strings, `Value`, and `ToolContext`, not `orbit-agent` transport objects.

`dispatch_v2_activity(...)` is the central per-activity entry. It emits `ActivityStarted` / `ActivityFinished` envelope events, then delegates by spec kind:

- `agent_loop` → HTTP or CLI path
- `groundhog` → Groundhog runner
- `deterministic` → host callback
- `shell` → direct subprocess execution

That keeps the dispatch tree readable while provider/session construction stays below the boundary.

---

## 7. Agent Loop Backend Paths

### 7.1 HTTP path

The HTTP path is driven by `agent_loop_driver.rs`. It:

- creates or reuses a `Session`
- constructs a `ToolContext`
- chooses a transport
- runs `orbit-agent`'s `AgentLoop`

This path is narrower than the schema: `Provider::has_http_transport()` currently returns true only for `claude`, so non-replay uses `AnthropicMessagesTransport`. `ORBIT_V2_REPLAY` and `ORBIT_V2_REPLAY_FIXTURE` provide scripted replay.

The allowlist is enforced in the loop engine on this path. A denied tool becomes a structural `DispatchError::ToolDenied` so the job retry wrapper can classify it as non-retryable.

After [T20260426-0526], completed HTTP loop outcomes become `InvocationTrace` records under the job run ID and step ID, including loop-body `session:` steps.

### 7.2 CLI path

The CLI path is driven by `cli_runner.rs`, added in [T20260419-0104]. The flow is:

1. Ask the host for the concrete CLI executor: command plus static executor args.
2. Build an `Agent` from `orbit-agent`.
3. Ask the retained CLI runtime for an `AgentInvocationSpec` containing provider-specific per-request args.
4. Emit the advisory `ToolAllowlistHarnessDelegated` event.
5. Resolve the subprocess cwd from runtime-owned workspace context.
6. Emit `CliInvocationStarted` with redacted argv, stdin blob ref, and resolved cwd.
7. Spawn the subprocess in that cwd with a wall-clock timeout.
8. Emit `CliInvocationFinished` with stdout/stderr blob refs and timeout state.
9. Parse the captured provider output with the existing Orbit response parser and persist its `InvocationTrace` through the host.

After [T20260426-2313], stdout/stderr readers emit line-level `tracing::info!` events while the child runs, carrying `provider`, `stream`, `job_run_id`, `task_id`, and `line`. After [T20260508-8], those events also carry `cwd` when the CLI subprocess has a resolved cwd. After [T20260426-2349], the default tracing subscriber redacts formatted output. The readers still retain original bytes for the existing audit/blob path, so run logs follow blob refs rather than the live feed.

Executor args are prepended before provider runtime args. For seeded Codex, the subprocess starts as `codex exec --json ...`, not the interactive TUI. [T20260423-0114] exposed the earlier command-only boundary.

After [T20260427-48], provider runtime args receive provider config through `V2RuntimeHost`. Static executor definitions keep command-shape flags (`exec --json`); dynamic Codex settings such as sandbox mode, side-write roots, and approval policy stay in the retained provider runtime. Codex approval policy is an exec-compatible config override, not the interactive-only `--ask-for-approval` flag.

After [T20260427-51], macOS CLI invocations declaring `sandbox: macos-sandbox-exec` run under `sandbox-exec -f <profile.sb> <provider> ...`. Orbit treats that SBPL profile as filesystem authority and neutralizes provider-native sandbox flags. After [T20260428-10], the profile grants Codex state (`$CODEX_HOME` or `$HOME/.codex`) plus side-write roots from provider config so inherited Orbit subprocesses can persist workflow state while project writes remain governed by `fsProfile`. After [T20260505-22], dispatch also runs `apply_provider_static_arg_fixups` before spawn, separately from sandbox neutralization. Today this only rewrites Claude's `--debug-file` value to `<claude_state_dir>/<basename>` so the log lands inside the already-writable state dir instead of `.orbit/**`, which the default policy denies.

After [T20260430-15], the CLI stdin envelope carries rendered activity input and durable `run_id` beside instruction, prompt, tools, and model. When input identifies one task, orbit-core embeds a canonical task snapshot with `input.workspace_path` / `input.repo_root` taking precedence over stored paths. After [T20260508-8], `backend: cli` also uses a shared workspace resolver for subprocess cwd: `input.workspace_path`, then `task.workspace_path`, then best-effort `ToolContext.workspace_root`. Declared input/task paths must already be directories; stale worktrees fail as `CliInvocationFailed` before `CliInvocationStarted` is emitted. `groundhog` delegates to the same resolver so task execution and attempt orchestration do not drift. After [T20260505-10], Orbit-managed CLI subprocesses receive `ORBIT_RUN_ID` plus an Orbit-managed run-context marker; `orbit tool run` requires both before it populates `ToolContext` reservation ownership. Direct manual CLI tool calls, including calls with only `ORBIT_RUN_ID`, remain unowned.

The older `AgentRuntime` trait and `providers/*_cli.rs` files are not deprecated leftovers; they are the shipped `backend: cli` implementation.

Just as important, Orbit does not enforce tool allowlists on this path today. It records the declared tool set as an advisory and delegates enforcement to the provider harness. This is a real semantic gap between `backend: http` and `backend: cli`.

---

## 8. Job Execution Semantics

The executor implementation lives under `crates/orbit-engine/src/activity_job/job_executor/` after [T20260509-2]. `mod.rs` owns the public exports and run entrypoint, while responsibility-focused child modules own audit projection, execution context, templating, step retry/recovery, target dispatch, parallel/fan-out/loop constructs, validation, and the small fan-out semaphore. The outward `activity_job::job_executor::{JobOutcome, execute_job, resolve_job_catalog_refs_for_execution, validate_job}` surface is unchanged.

### 8.1 Template rendering and pipeline context

The executor exposes outputs as `{{ steps.<id>.output.* }}`. Initial context follows the §3 merge contract: object caller input overlays object `job.default_input`, while `null` and non-object inputs keep their special cases. Step `default_input` is rendered recursively; strings that parse as JSON convert back into `Value`.

`fan_out` workers see `{{ item }}` / `{{ input.item }}`. Loop bodies see `{{ input.iteration }}`.

#### Agent-step state handoff via `orbit.state.*`

Agents running inside an activity step pass durable data to later steps through `orbit.state.*`, not through the step's response payload. The contract:

- `orbit.state.get` reads the persisted pipeline snapshot.
- `orbit.state.set` writes this step's output for the engine to merge after the step finishes.
- Once needed fields are written to `orbit.state`, the activity itself usually has no structured response-payload requirement.
- `orbit.task.update` stays the right tool for task artifacts (`execution_summary`, `pr_status`, comments, lifecycle state). That is task persistence, not pipeline-state handoff.
- `orbit.state.*` is only callable when the activity allowlist includes those tools. Currently only [step_failure_recovery](../../../crates/orbit-core/assets/activities/step_failure_recovery.yaml) grants them; other activities thread data through `{{ steps.<id>.output.* }}` or purpose-built tools (e.g. `orbit.duel.plan.winner`).

For `run_command` or any shell-based step, there is no implicit structured output path beyond `exit_code`. If the command must feed downstream steps, have it invoke `orbit.state.set` explicitly. Downstream jobs read the persisted state instead of parsing shell stdout.

### 8.2 `when` and `retry`

`when` is evaluated once, before retry. A skipped step is a successful no-op and does not retry.

The retry wrapper re-runs the whole step body up to `max_attempts`, with exponential or linear backoff. Some errors bypass retry:

- tool denial
- unknown deterministic action
- shell allowlist violation
- host-required / backend-resolution structural errors
- job validation errors

That rule comes straight from `DispatchError::is_non_retryable()`.

### 8.3 `parallel`

Parallel branches run under `std::thread::scope`. Join policy is:

- `all`
- `any`
- `quorum { n }`

The executor emits `StepJoin` with per-branch outcomes. If the join policy fails and any branch produced a structural error, the first error is surfaced instead of only `success: false`.

### 8.4 `fan_out` / `fan_in`

`fan_out.items` is template-rendered into an array. Workers run concurrently behind a counting semaphore, so `max_workers` is a true concurrency bound, not just metadata. `fan_in.collect` can persist the ordered worker outputs under a separate pipeline key in addition to the step id itself.

Workers use isolated pipeline/session maps. The validator rejects any worker template with `session:` because concurrent workers would otherwise share one mutable `Session`.

### 8.5 `loop`

A loop runs either:

- once per rendered `items` entry
- or up to `max_iterations` when `items` is absent

The body runs before `break_when`, so steps can populate fields the break expression reads. If `items` exceeds `max_iterations`, execution fails structurally instead of truncating.

### 8.6 Persisted state for v2 job runs

Persisted pipeline runs (`orbit run ship`, `ship-auto`, `duel-plan`, `orbit.pipeline.invoke` + `orbit.pipeline.wait`) go through `pipeline_run.rs`. Direct v2 runs (`orbit job run <job-id-or-yaml>`) also create durable `JobRun` bundles after [T20260423-2004-4] under `state/job-runs/<job_id>/<run_id>/`, so `orbit run history -j <job_id>` and `orbit run show <run_id>` can inspect the returned ID. Workflow-specific `orbit run <workflow> list/show` aliases were removed in [T20260425-2010], and duplicate job-level aliases in [T20260426-0742].

Before [T20260423-0445], early v2 failures could leave `steps: []` and no surfaced `error_message`. The current contract is:

- if a persisted v2 pipeline fails and no recorded step already carries error detail, the pipeline worker writes a synthetic failed `JobRunStep`
- if a direct v2 run succeeds, the direct-run wrapper writes a synthetic successful `JobRunStep` containing the final pipeline snapshot
- that synthetic step uses `target_type: job` and `target_id: <job_id>`
- the step's `error_message` carries the concrete executor error (or a fallback `success=false` summary for message-carrying non-success results)

This operator-surface repair keeps `orbit run ship --json`, direct `orbit job run`, `orbit run history`, and `orbit run show` actionable without adding a second run-level error channel.

After [T20260430-27], `orbit run ship-auto` also interprets the parent `task_auto_pipeline` snapshot for operator output. Text and JSON modes keep the persisted run state and exit-code semantics, but add `workflow_status` labels: `empty_backlog`, `gated_noop`, `gate_waiting`, `gate_failed`, and `completed`. `empty_backlog` means no candidates and no exclusions. `gated_noop` means zero dispatched bundles with one or more `list_backlog.excluded` entries. `gate_waiting` means a child `task_gate_pipeline` run is still pending/running or the parent wait timed out while the child remains active. `gate_failed` means a child gate run reached a failed or cancelled state. The output also carries dispatched bundle count, excluded task count, exclusion reasons, blocker summaries, and child gate run status so operators do not have to run `orbit run show` merely to tell no backlog from lock-gated work. After [T20260430-30], default text renders that data as labeled multi-line operator output, while `--json` remains the stable machine-readable surface with raw status fields.

After [T20260505-8], active job runs can be cancelled through the same durable run surface. `pending` and `running` runs transition to `cancelled`; terminal runs remain immutable. Pending cancellation only rewrites the run bundle and pipeline snapshot, so a later pipeline worker observes `cancelled` and exits without claiming the run. Running cancellation first validates the stored owner PID start-time token, then signals the owner process group on Unix with a bounded graceful period and `SIGKILL` escalation. `JobRunCancelled` audit payloads include run id, previous/final state, actor/source, whether signaling was attempted, and the signal outcome.

After [T20260505-21], whole-run replay creates a fresh durable `JobRun` from an existing run's persisted input and the current catalog job definition. Replay never mutates the source run bundle or source audit envelope; lineage lives on the new run as `retry_source_run_id` and in the new v2 `run.started` audit envelope. This is intentionally whole-run only: every step executes from step 0, and changed or deleted job YAML is resolved at replay time rather than read from a source-run snapshot.

The loop shares one pipeline map and session map across iterations, which makes cross-iteration `session:` meaningful.

### 8.7 Invocation metrics

`orbit metrics` reads knowledge usage from job-run state and agent/tool usage from the SQLite invocation store. It does not scrape `.orbit/state/audit/v2_loop/` or diagnostics JSONL.

V2 jobs persist invocation traces explicitly after [T20260426-0526]. `DispatchOutcome` carries optional trace data; the executor attaches run and step IDs; orbit-core stores canonical agent/model names plus task IDs from rendered input and refreshes the token scoreboard.

For `backend: cli`, the trace comes from the provider's structured stdout using the same parser that validates Orbit response envelopes. For the HTTP loop path, the trace is derived from `LoopOutcome` usage and tool-call names.

### 8.8 Run trace inspection

`orbit run show`, `logs`, `events`, and `trace` inspect already-scheduled runs and resolve an omitted run ID to the most recent run.

After [T20260426-0709], `orbit run show <run> -s <id>` treats the v2 envelope's activity DAG `step.id` as primary. This matters because durable v2 runs may store a synthetic job-level `JobRunStep`, while the envelope records actual YAML step IDs. `JobRunStep.target_id` and numeric `step_index` remain fallbacks.

After [T20260426-0705], `orbit run events <run>` reads the v2 envelope chronologically and filters by step ID or event type. `orbit run trace <run>` renders the parent/child tree from `event_id` and `parent_event_id`. JSON mode is deterministic.

The CLI does not own envelope storage. `orbit-core` exposes accessors for v2 audit events and CLI invocation records, including derived step IDs and blob-backed stdout/stderr, keeping storage knowledge with the runtime layer.

### 8.9 Workflow worktree base synchronization

Task-shipping workflows that create worktrees (`task_pr_pipeline`, `task_local_pipeline`, and callers such as `task_auto_pipeline`) default `base_sync` to `remote` after [T20260427-45]. Remote mode fetches `origin/<base_branch>` and creates, resets, compares, and rebases task branches against that remote-tracking ref; it does not mutate the repo root checkout or prefer stale local base branches.

Direct callers can set `base_sync: local` for local-only repos or unpublished base branches. That mode resolves the local base ref and skips origin fetch.

### 8.10 Workflow task admission

After [T20260428-8], task-starting workflows own explicit admission instead of relying on generic task updates. `worktree_setup` and `run_planning_duel` accept `proposed`, `friction`, `backlog`, `rejected`, and `archived` tasks into `in-progress`; existing `in-progress` tasks are idempotent retry inputs.

This path stays separate from `orbit.task.update` and generic deterministic metadata stamping. Direct task updates keep the non-empty-plan guard, and workflow admission records system-actor lifecycle history while preserving friction-bounty accounting for `friction -> in-progress`.

Planning-duel writeback now reports `task_status: "in-progress"` instead of `status_unchanged`; the plan artifact still lands through `planning_duel_resolved`.

### 8.11 Task PR handoff summaries

`task_pr_pipeline` sends the selected task IDs to `pr_open` as `completed_task_ids`. Before `pr_open` pushes or creates the pull request, the deterministic action reloads each task record, checks that the task still belongs to the batch, confirms it can enter review, and requires a meaningful persisted `execution_summary` for every completed task. Empty, whitespace-only, and explicit placeholder summaries fail the PR step with an error naming the task id; generated default PR bodies also omit placeholder summary details blocks. When callers pass a non-empty `body`, `pr_open` preserves that body verbatim after the same durable-summary guard passes. This handoff contract was tightened in [T20260430-31].

After [T20260508-3], generated one-task PR bodies render the task contract first: `## Task`, optional collapsed `## Execution Summary`, `## Validation`, then `## Branch Freshness`. The task section includes the task link, description, and plain-bullet acceptance criteria so reviewers can see the requested work beside the implementation summary. Multi-task callers keep the legacy `## Tasks` plus files-changed layout until those paths are retired.

---

## 9. Filesystem Policy and `fsProfile`

Both `ActivityV2` and `TargetStep` can attach an `fsProfile`. orbit-core uses `tool_context_for_activity(...)` to build the policy-aware `ToolContext`, and `V2AuditWriter` can attach filesystem audit logging so read/write denials appear in the envelope.

Runtime/CLI enforcement landed in [T20260419-0503]. `fsProfile` is therefore part of the activity/job contract, not a CLI presentation detail.

One subtlety: profile attachment happens at two layers.

- An activity asset may declare its own `fsProfile`.
- A target step may override or supply one around an inlined activity spec.

Readers must distinguish "profile on the reusable activity" from "profile on this call site."

---

## 10. Legacy Surfaces and Retention Boundaries

This feature spans a migration, so the retained surfaces are explicit.

### 10.1 Retention Table

| Surface | Current status | Rationale |
|---------|----------------|-----------|
| `schemaVersion: 1` activity/job assets | Retired | Load-time hard error after [T20260419-2156]. |
| v2 `agent_loop` HTTP path | Kept | Canonical typed runtime path from [T20260418-2010]. |
| v2 `agent_loop` CLI path | Kept | Implemented by the retained `AgentRuntime` trait and `providers/*_cli.rs` after [T20260419-0104]. |
| `TargetRef` authoring form | Kept at authoring/load time only | Human-friendly YAML surface; resolved away before execution since [T20260418-2019]. |
| v1 `crate::job_runner` | Kept | Older sequential/DAG runtime still exists beside the v2 executor; Phase 3 was additive in [T20260418-2018]. |
| Seeded reference activities and jobs | Kept | They act as runnable contracts and examples, and were moved into init seeding in [T20260419-2347]. |
| Groundhog as a dedicated activity kind | Kept | Explicit sibling activity after [T20260420-0510-2], not an `agent_loop` toggle. |

### 10.2 Seeded Assets in Practice

Seeded assets are part of the design. Today they include:

- small reference activities such as `agent_loop_reference` and `agent_loop_cli_reference`
- control-plane jobs such as `task_gate_pipeline`
- higher-level dispatch workflows such as `task_auto_pipeline`, `task_epic_pipeline`, and `job_duel_plan_pipeline`

The gate/auto/epic assets from [T20260419-0622-3], [T20260419-0623], and [T20260419-0623-2] exercise real v2 constructs:

- `loop + break_when`
- `fan_out + fan_in`
- cross-iteration `session:` binding
- deterministic child-job dispatch

That seeded corpus is Activity / Job's executable reference documentation.

---

## 11. Concerns & Honest Limitations

### 11.1 Provider typing is broader than provider wiring

The `Provider` enum names `claude`, `codex`, `gemini`, `ollama`, and `openai_compat`, but HTTP transport currently wires only `claude`. The schema is broader than the runtime.

### 11.2 Tool enforcement differs materially by backend

HTTP agent loops enforce the tool allowlist inside Orbit. CLI agent loops emit an advisory event and rely on the provider harness.

### 11.3 Some structural controls are still literals

`LoopBlock.max_iterations` and `FanOutBlock.max_workers` are structural `u32`s, not templated expressions, so workflows must fork YAML to change them dynamically.

### 11.4 Validation is split across phases

Some bad shapes fail at load time, some at job preflight, and some during dispatch. The "where will this fail?" answer is not yet uniform.

### 11.5 The audit story is powerful but split

The v2 envelope tree lives in `.orbit/state/audit/v2_loop/`, HTTP loop details materialize lazily in `.orbit/state/audit/loop/`, and payload blobs live in `.orbit/state/audit/blobs/`. Reviewers still need to know the split layout. [T20260426-0519] moved these traces under `.orbit/state/` so top-level `.orbit/` stays for config, resources, tasks, graph artifacts, and the SQLite command-audit database; [T20260506-2] stopped creating empty loop JSONL files for runs with no loop-level events.

### 11.6 The substrate still leaks into the public product story

README frames tasks, jobs, and activities as substrate. The CLI and seeded assets still expose this layer because Orbit needs it to operate today.

### 11.7 Nearby comments still carry migration-era drift

Some module prose still reflects earlier phase names or pass ordering. orbit-core entrypoints and executor behavior are authoritative.

### 11.8 Historical run inspection belongs to the run surface

Read-only history does not need the same dependencies as live execution. [T20260423-0447] kept retired workflow runs observable without live assets, [T20260425-2010] removed workflow-specific history browsers, and [T20260426-0742] removed duplicate job-level inspection aliases. Current inspection belongs to `orbit run history -j <job_id>` and `orbit run show <run_id>`; `orbit job` is for catalog browsing and direct execution.

---

## Task References

- **[T20260413-0141]** — Support step default inputs in jobs.
- **[T20260418-2010]** — Add the first v2 activity runtime scaffolding.
- **[T20260418-2018]** — Add `JobV2` DAG constructs (`parallel`, `fan_out`, `loop`, `retry`, `when`).
- **[T20260418-2019]** — Add v2 activity name resolution and pipeline skeleton assets.
- **[T20260418-2143]** — Wire `V2RuntimeHost` in orbit-core and add `orbit activity run-v2`.
- **[T20260418-2210]** — Reshape `V2RuntimeHost` to keep `orbit-agent` types out of orbit-core.
- **[T20260419-0002]** — Add `workspace_path` provenance to the v2 audit envelope.
- **[T20260419-0104]** — Add `backend: cli` dispatch for v2 `agent_loop`.
- **[T20260419-0339]** — Add v2 job kinds to the job catalog.
- **[T20260419-0503]** — Enforce `fsProfile` rules across runtime and CLI surfaces.
- **[T20260419-0622-3]** — Add `task_gate_pipeline`.
- **[T20260419-0623]** — Add `task_auto_pipeline`.
- **[T20260419-0623-2]** — Add `task_epic_pipeline`.
- **[T20260419-2156]** — Retire v1 assets and drop the transitional v2 naming.
- **[T20260419-2347]** — Seed activities and workflows on `orbit init`.
- **[T20260420-0510-2]** — Add the Groundhog v1 activity runner.
- **[T20260421-0542-2]** — Add pre-gate lock-overlap exclusion attribution to `list_backlog_tasks`.
- **[T20260423-0114]** — Expose the `backend: cli` executor-args gap during a local task ship run.
- **[T20260423-0445]** — Merge object-valued job defaults over explicit run input and persist synthetic failed job steps for early v2 pipeline failures.
- **[T20260423-0447]** — Restore usable `orbit run duel` read-only surfaces after duel workflow retirement.
- **[T20260423-2004-4]** — Persist direct v2 `orbit job run` executions into durable job-run records and state.
- **[T20260425-0204]** — Make v2 job catalog discovery honor workspace-over-global `MergeByKey` precedence.
- **[T20260425-2010]** — Refactor `orbit run` task workflow commands and remove workflow-specific history browsers.
- **[T20260426-0047]** — Make v2 activity catalog discovery honor workspace-over-global `MergeByKey` precedence and remove the public `orbit activity run` command.
- **[T20260426-0526]** — Restore v2 job invocation trace persistence so `orbit metrics` can report agent and tool usage.
- **[T20260426-0519]** — Move file-backed activity/job audit traces under `.orbit/state/audit`.
- **[T20260426-0705]** — Expose v2 run audit events through `orbit run events` and `orbit run trace`.
- **[T20260426-0709]** — Align run step selectors on activity `step.id` and move CLI invocation log reading behind orbit-core runtime accessors.
- **[T20260426-0742]** — Remove duplicate job-level run inspection aliases and keep run inspection under `orbit run`.
- **[T20260426-2313]** — Stream CLI subprocess stdout/stderr through structured tracing events while retaining the existing audit/blob path.
- **[T20260426-2349]** — Move CLI tracing output redaction from `cli_runner` call sites into the default tracing formatter layer.
- **[T20260427-34]** — Add seeded pipeline success guards so non-succeeded child runs fail parent shipment workflows.
- **[T20260427-36]** — Align task-gate reservation TTL with the child dispatch wait budget.
- **[T20260427-38]** — Treat review as a shipped stop state for epic automation.
- **[T20260427-45]** — Use freshly fetched remote base refs for default task-shipping worktrees.
- **[T20260427-48]** — Thread provider config into the v2 CLI backend and keep Codex dynamic flags exec-compatible.
- **[T20260427-51]** — Wrap cli-backend agent invocations in `sandbox-exec` on macOS.
- **[T20260428-8]** — Add explicit workflow admission for task-starting workflows and remove the plan prerequisite from those workflow starts.
- **[T20260428-10]** — Allow Codex CLI state writes under the macOS sandbox.
- **[T20260430-15]** — Embed task-aware input and run context in backend: cli agent envelopes.
- **[T20260430-19]** — Shorten the Activity / Job design docs while preserving required structure.
- **[T20260430-26]** — Release task-gate reservations after terminal child shipment runs and expose active reservations through the lock view.
- **[T20260430-27]** — Make `ship-auto` output distinguish empty backlog, gated no-op, and waiting gate children.
- **[T20260430-30]** — Make `ship-auto` default text output human-readable while preserving JSON fields.
- **[T20260430-31]** — Require populated execution summaries before opening task PRs.
- **[T20260505-2]** — Admit accepted backlog friction reports in automatic backlog listing.
- **[T20260505-8]** — Add dashboard/runtime controls to cancel active job runs.
- **[T20260505-10]** — Release run-owned task lock reservations through engine-owned terminal cleanup and reserve-pressure reconciliation.
- **[T20260505-21]** — Add whole-run replay with `retry_source_run_id` lineage and current-definition semantics.
- **[T20260506-2]** — Lazily materialize loop audit JSONL files only when loop-level events are emitted.
- **[T20260508-8]** — Resolve backend: cli subprocess cwd from workspace context and record it in audit/tracing.
- **[T20260509-2]** — Split the v2 job executor into responsibility-focused modules without changing runtime behavior.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
