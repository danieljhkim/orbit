# Activity / Job — Design

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-04-23

This document describes the shipped Activity / Job substrate as it exists today across `orbit-common`, `orbit-engine`, `orbit-core`, and `orbit-cli`: asset shape, load-time normalization, dispatch boundaries, backend semantics, DAG execution, audit, and the legacy edges that still matter. See [1_overview.md](./1_overview.md) for the feature's purpose and [3_vision.md](./3_vision.md) for forward-looking questions.

---

## 1. Asset Shape and Two-Pass Loading

Activity / Job assets are `schemaVersion: 2` YAML envelopes with:

- `kind: Activity` or `kind: Job`
- `metadata.name`
- typed `spec`

The loader in `crates/orbit-common/src/types/activity_job/asset_loader.rs` reads the schema header first, then parses the full envelope into `ActivityV2` or `JobV2`. That two-pass shape arrived with the first v2 activity runtime scaffolding in [T20260418-2010].

The important current contract is that `schemaVersion: 1` is not "legacy but tolerated." It is retired. The loader returns a structural error for version 1 after [T20260419-2156]. `kind` mismatches are also structural errors, so an activity file can never accidentally dispatch as a job or vice versa.

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

Groundhog intentionally does not reuse the exact serialized `AgentLoopSpec` shape. It has its own `GroundhogSpec`, but `as_agent_loop_spec()` projects it into an HTTP-backed agent loop when the runner needs the shared transport path. That sibling activity kind landed in [T20260420-0510-2].

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

Phase 3 also made step-local input layering explicit. Job-level `default_input` fills the run input only when the caller passes `null`, and step-level `default_input` is recursively template-rendered before dispatch. Support for step default inputs landed earlier in [T20260413-0141] and was pulled into the v2 DAG path in [T20260418-2018].

---

## 4. Load-Time Normalization Pipeline

The code does not dispatch the raw YAML shape. orbit-core normalizes it first.

For a single activity run:

1. Read YAML from disk.
2. Parse via `load_activity_asset(...)`.
3. Resolve `backend: auto` to a concrete backend.
4. Build audit sinks and run id.
5. Dispatch the concrete `ActivityV2Spec`.

For a job run:

1. Read YAML from disk.
2. Parse via `load_job_asset(...)`.
3. Build the activity catalog from seeded/workspace activity directories.
4. Resolve every `target: activity:<name>` into a concrete `TargetStep`.
5. Resolve every `backend: auto` in the now-concrete step tree.
6. Reject loop-body `session:` bindings that resolve to `backend: cli`.
7. Build audit sinks and run id.
8. Execute the normalized `JobV2`.

The target-ref pass was added in [T20260418-2019]. The concrete backend resolution and `run-v2` entrypoints were wired in [T20260418-2143]. The CLI backend path and the HTTP-only loop/session rejection tightened this load-time contract in [T20260419-0104].

One nuance worth naming: some module comments still describe older Phase ordering. The authoritative behavior is the orbit-core call path above in `crates/orbit-core/src/command/job_v2.rs`.

---

## 5. Backend Resolution and Constraint Rules

`Backend::Auto` is never supposed to reach dispatch. orbit-core resolves it once per run using the precedence chain implemented in `backend_resolver.rs`:

1. `--backend=<value>`
2. `ORBIT_BACKEND`
3. `[runtime] backend = "<value>"` in config
4. hard-coded fallback `http`

If any intermediate tier literally says `auto`, the resolver folds it to the hard-coded fallback so the dispatcher only sees `http` or `cli`. That "resolve once, dispatch concrete" rule was introduced with the `run-v2` entrypoints in [T20260418-2143] and hardened for the CLI path in [T20260419-0104].

The second rule is the HTTP-only feature constraint. Today the only public item in that list is loop-body cross-iteration `session:` binding. `validate_job_loop_session_backends(...)` rejects a step inside `loop:` that declares `session:` while resolving to `backend: cli`. The error text intentionally names the feature and the fix, because the job has not started yet; there is no useful retry path.

The third rule is no silent provider fallback. `backend: http` against an unwired provider fails as `UnwiredHttpTransport` rather than quietly launching a CLI runtime. That choice matters because providers and backends are separate choices in the schema, not aliases.

The prescriptive contract for this area lives in [specs/backend-resolution.md](./specs/backend-resolution.md).

---

## 6. Engine-Core Boundary

Activity / Job is the seam where orbit-core hands work to orbit-engine without taking a dependency on `orbit-agent` types.

`V2RuntimeHost` is the key boundary. orbit-core implements it and supplies four services back into the engine:

- run a deterministic action by name
- source an API key for a provider
- resolve a provider's CLI command
- build `ToolContext` for an activity, including policy and filesystem audit hooks

That host wiring arrived in [T20260418-2143]. The cleanup in [T20260418-2210] made the boundary primitive on purpose: the engine receives strings, `Value`, and `ToolContext`, not `orbit-agent` transport objects.

`dispatch_v2_activity(...)` is the central per-activity entry. It emits `ActivityStarted` / `ActivityFinished` envelope events, then delegates by spec kind:

- `agent_loop` → HTTP or CLI path
- `groundhog` → Groundhog runner
- `deterministic` → host callback
- `shell` → direct subprocess execution

That makes the dispatch tree readable in one place while keeping provider/session construction below the boundary.

---

## 7. Agent Loop Backend Paths

### 7.1 HTTP path

The HTTP path is driven by `agent_loop_driver.rs`. It:

- creates or reuses a `Session`
- constructs a `ToolContext`
- chooses a transport
- runs `orbit-agent`'s `AgentLoop`

Today that transport path is narrower than the schema surface suggests. `Provider::has_http_transport()` currently returns true only for `claude`, so the non-replay path uses `AnthropicMessagesTransport`. `ORBIT_V2_REPLAY` and `ORBIT_V2_REPLAY_FIXTURE` provide a scripted replay transport for smoke runs and loop convergence tests.

The allowlist is enforced in the loop engine on this path. A denied tool becomes a structural `DispatchError::ToolDenied` so the job retry wrapper can classify it as non-retryable.

### 7.2 CLI path

The CLI path is driven by `cli_runner.rs`, added in [T20260419-0104]. The flow is:

1. Ask the host for the concrete CLI command.
2. Build an `Agent` from `orbit-agent`.
3. Ask the retained CLI runtime for an `AgentInvocationSpec`.
4. Emit the advisory `ToolAllowlistHarnessDelegated` event.
5. Emit `CliInvocationStarted` with redacted argv and stdin blob ref.
6. Spawn the subprocess with a wall-clock timeout.
7. Emit `CliInvocationFinished` with stdout/stderr blob refs and timeout state.

The important retention boundary is that the older `AgentRuntime` trait and `providers/*_cli.rs` files are not deprecated leftovers. They are the shipped implementation of `backend: cli`.

Just as important, Orbit does not enforce tool allowlists on this path today. It records the declared tool set as an advisory and delegates enforcement to the provider harness. This is a real semantic gap between `backend: http` and `backend: cli`.

---

## 8. Job Execution Semantics

### 8.1 Template rendering and pipeline context

The executor wraps pipeline outputs so templates read them as `{{ steps.<id>.output.* }}`. Step `default_input` is rendered recursively through the template engine; strings that parse as JSON are converted back into `Value`, so booleans, numbers, arrays, and objects can flow forward without remaining strings.

`fan_out` workers additionally see `{{ item }}` / `{{ input.item }}`. Loop bodies additionally see `{{ input.iteration }}`.

### 8.2 `when` and `retry`

`when` is evaluated once, before retry. A skipped step is a successful no-op and does not retry.

The retry wrapper re-runs the whole step body up to `max_attempts`, using exponential or linear backoff. Some errors bypass retry entirely:

- tool denial
- unknown deterministic action
- shell allowlist violation
- host-required / backend-resolution structural errors
- job validation errors

That rule comes straight from `DispatchError::is_non_retryable()`.

### 8.3 `parallel`

Parallel branches run under `std::thread::scope`. The join policy is one of:

- `all`
- `any`
- `quorum { n }`

The executor emits a `StepJoin` event with per-branch outcomes. If the join policy fails and at least one branch produced a structural error, the first error is surfaced instead of returning only `success: false`.

### 8.4 `fan_out` / `fan_in`

`fan_out.items` is template-rendered into an array. Workers run concurrently behind a counting semaphore, so `max_workers` is a true concurrency bound, not just metadata. `fan_in.collect` can persist the ordered worker outputs under a separate pipeline key in addition to the step id itself.

Workers execute with isolated pipeline/session maps. That isolation is why the validator rejects any worker template that names a `session:` binding: concurrent workers would otherwise share one mutable `Session`.

### 8.5 `loop`

A loop runs either:

- once per rendered `items` entry
- or up to `max_iterations` when `items` is absent

The loop body runs before `break_when` is evaluated, so body steps can populate the pipeline fields the break expression reads. If `items` expands beyond `max_iterations`, the executor fails structurally instead of silently truncating the iteration set.

The loop shares the same pipeline map and session map across iterations. That is what makes cross-iteration `session:` binding meaningful in the first place.

---

## 9. Filesystem Policy and `fsProfile`

Both `ActivityV2` and `TargetStep` can attach an `fsProfile`. orbit-core uses `tool_context_for_activity(...)` to build the policy-aware `ToolContext`, and `V2AuditWriter` can attach a filesystem audit logger so read/write denials are reflected in the envelope event stream.

The runtime/CLI enforcement tightening landed in [T20260419-0503]. The practical effect is that `fsProfile` is part of the activity/job contract, not an optional CLI presentation detail.

One subtlety: profile attachment happens at two layers.

- An activity asset may declare its own `fsProfile`.
- A target step may override or supply one around an inlined activity spec.

This is useful, but it means readers have to distinguish "profile on the reusable activity" from "profile on the place this activity is called."

---

## 10. Legacy Surfaces and Retention Boundaries

This feature spans a migration, so the docs need to say plainly what is gone, what is still intentionally present, and why.

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

The seeded asset set is part of the design, not just repo clutter. Today it includes:

- small reference activities such as `agent_loop_reference` and `agent_loop_cli_reference`
- control-plane jobs such as `task_gate_pipeline`
- higher-level dispatch workflows such as `task_auto_pipeline` and `task_epic_pipeline`

The gate/auto/epic assets added in [T20260419-0622-3], [T20260419-0623], and [T20260419-0623-2] are especially useful because they exercise the real v2 constructs instead of describing them abstractly:

- `loop + break_when`
- `fan_out + fan_in`
- cross-iteration `session:` binding
- deterministic child-job dispatch

That seeded corpus is the closest thing Activity / Job has to executable reference documentation.

---

## 11. Concerns & Honest Limitations

### 11.1 Provider typing is broader than provider wiring

The `Provider` enum names `claude`, `codex`, `gemini`, `ollama`, and `openai_compat`, but the HTTP transport path currently wires only `claude`. The schema reads more general than the runtime is.

### 11.2 Tool enforcement differs materially by backend

HTTP agent loops enforce the tool allowlist inside Orbit. CLI agent loops emit an advisory event and rely on the provider harness. Same field, different enforcement model.

### 11.3 Some structural controls are still literals

`LoopBlock.max_iterations` and `FanOutBlock.max_workers` are structural `u32`s, not templated expressions. Real workflows like `task_auto_pipeline` therefore have to fork YAML to change those limits dynamically.

### 11.4 Validation is split across phases

Some bad shapes are rejected at load time, some at job preflight validation, and some during dispatch. This is pragmatic, but it means the "where will this fail?" answer is not yet uniform.

### 11.5 The audit story is powerful but split

The v2 envelope tree lives in `.orbit/audit/v2_loop/`. The loop-engine HTTP details and blobs live in the older loop sink. That split is intentional, but reviewers still need to know two storage layouts.

### 11.6 The substrate still leaks into the public product story

README already frames tasks, jobs, and activities as substrate rather than the long-term product interface. The code agrees, but the CLI and seeded assets still expose this layer directly because Orbit needs it to operate today.

### 11.7 Nearby comments still carry migration-era drift

Most code comments are accurate, but some module prose still reflects earlier phase names or pass ordering. When there is tension, orbit-core entrypoints and executor behavior are the authoritative source.

### 11.8 Historical run inspection can outlive seeded assets

Read-only history surfaces do not always have the same dependency shape as live execution. After [T20260423-0447], `orbit run duel list` and latest `show` intentionally read stored run bundles without requiring the original duel job asset to remain seeded, because retirement of an executable surface must not also erase its observability trail.

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
- **[T20260423-0447]** — Restore usable `orbit run duel` read-only surfaces after duel workflow retirement.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
