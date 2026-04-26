# Activity / Job — Decisions

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-04-26

This ADR log records the decisions that define the current Activity / Job substrate. Entries are append-only and stay in place when later ADRs supersede them. See [1_overview.md](./1_overview.md) for the feature summary, [2_design.md](./2_design.md) for the current implementation, and [3_vision.md](./3_vision.md) for the questions that may force more decisions.

---

## ADR-001 — `schemaVersion: 2` is the canonical activity/job surface

**Status:** Accepted · 2026-04 · [T20260419-2156]

**Context.** The migration period where v1 and v2 both existed made every doc and runtime path harder to reason about. Orbit needed one canonical schema family, not a permanently dual parser.

**Decision.** Treat `schemaVersion: 2` as the canonical activity/job surface and fail fast on version 1 assets at load time.

**Consequences.**
- The runtime now has one typed surface to document and validate.
- Migration failures happen structurally, before a run starts.
- Cost: old assets stop limping along; migration work becomes mandatory instead of gradual.

## ADR-002 — Keep activity/job types in `orbit-common`, not in engine/core

**Status:** Accepted · 2026-04 · [T20260419-2014], [T20260418-2210]

**Context.** The v2 runtime shape has to be shared across core, engine, CLI, and seeded assets. Letting the type surface live inside one runtime layer would either duplicate schema definitions or leak higher-layer dependencies downward.

**Decision.** Keep the activity/job type family in `orbit-common`, and keep the engine/core boundary primitive enough that orbit-core does not name `orbit-agent` types.

**Consequences.**
- Parsing, validation, dispatch, and CLI display all talk about the same Rust types.
- The engine/core seam stays explicit and reviewable.
- Cost: `orbit-common` now owns a wider slice of runtime vocabulary and has to stay disciplined about not accreting behavior.

## ADR-003 — Resolve `backend: auto` once, before dispatch

**Status:** Accepted · 2026-04 · [T20260418-2143], [T20260419-0104]

**Context.** Allowing `Backend::Auto` to leak into the dispatcher would make the execution path depend on call-site behavior and ambient environment deep into runtime code. That is hard to audit and easy to get wrong.

**Decision.** Resolve `backend: auto` once per run using flag → env → config → default precedence, rewrite the parsed asset in place, and require all downstream code to see only concrete backends.

**Consequences.**
- Dispatch and validation reason about one concrete backend choice.
- CLI and HTTP paths can reject incompatible shapes structurally.
- Cost: callers must remember to run the normalization pass before dispatch, and any missed call site fails as a structural bug.

## ADR-004 — `target: activity:<name>` is authoring sugar, not an execution primitive

**Status:** Accepted · 2026-04 · [T20260418-2019]

**Context.** Human-authored jobs benefit from referencing named activities. The executor, however, should not have to look up activities by name mid-run or carry catalog semantics through execution.

**Decision.** Keep `TargetRef` at the authoring/load layer only, and resolve every reference to a concrete `TargetStep` before execution starts.

**Consequences.**
- Human-authored YAML stays readable and reusable.
- The executor operates on one target shape.
- Cost: the load path owns more normalization logic, and stale refs fail before dispatch instead of being lazily recoverable.

## ADR-005 — Cross-iteration `session:` binding is a loop-scoped HTTP-only feature

**Status:** Accepted · 2026-04 · [T20260418-2018], [T20260419-0104], [T20260419-0623-2]

**Context.** Reusing one provider conversation across loop iterations is valuable for orchestrator-style steps, but a shared `Session` is mutable and not safe to spread across arbitrary concurrent shapes. The CLI backend also does not expose the same session semantics as the HTTP loop engine.

**Decision.** Support named `session:` bindings only where the runtime can preserve one HTTP `Session` across loop iterations, reject loop-body `session + cli` at load time, and reject concurrent shapes that would share a session unsafely.

**Consequences.**
- Loop orchestrators such as `task_epic_pipeline` get persistent conversation history.
- Parallel/fan-out validation stays simple and explicit.
- Cost: session reuse becomes a narrowly scoped feature instead of a general-purpose memory layer.

## ADR-006 — Keep the retained CLI runtimes as the implementation of `backend: cli`

**Status:** Accepted · 2026-04 · [T20260419-0104], [T20260418-2210]

**Context.** Orbit already had mature CLI-provider runtimes. Replacing them with a brand-new v2-only path would have duplicated provider integration work and risked regressing the CLI story just to satisfy an architectural cleanup.

**Decision.** Keep `AgentRuntime` and `providers/*_cli.rs` as the implementation of `backend: cli`, and route v2 CLI dispatch through them rather than deleting them as legacy.

**Consequences.**
- The v2 runtime gets a CLI backend without a second provider-integration stack.
- The engine/core boundary remains clean because orbit-core only supplies primitive CLI executor fields, not orbit-agent transport objects.
- Cost: the feature now has materially different semantics between HTTP and CLI, especially around tool enforcement.

## ADR-007 — Treat the v2 audit envelope as a separate tree layered over loop-level audit

**Status:** Accepted · 2026-04 · [T20260419-0002]

**Context.** Activity/job execution needs run/step/activity structure that the lower-level loop sink does not provide. At the same time, the loop sink already owns full HTTP transcript and blob persistence.

**Decision.** Add a separate v2 audit envelope tree with `parent_event_id`, `run_id`, `agent_identity`, and `workspace_path`, and let it point at rather than replace the underlying loop-level sink.

**Consequences.**
- Reviewers can traverse runs by job/step/activity structure without losing raw loop detail.
- Workspace provenance becomes queryable at the envelope layer.
- Cost: audit review now spans two related storage layouts instead of one.

## ADR-008 — Seed reference activities and jobs as load-bearing runtime contracts

**Status:** Accepted · 2026-04 · [T20260419-2347], [T20260419-0622-3], [T20260419-0623], [T20260419-0623-2]

**Context.** Activity/job semantics are easier to trust when they exist in real assets, not only in types and tests. Orbit also needs a usable default runtime after `orbit init`.

**Decision.** Treat seeded activities and jobs as part of the runtime contract: they are default assets, examples, and executable reference surfaces all at once.

**Consequences.**
- New workspaces start with a real control-plane corpus instead of empty directories.
- Complex constructs such as `fan_out`, `loop`, and `session:` live in reviewable YAML, not just in Rust tests.
- Cost: seeded assets become part of the public maintenance burden and can drift if docs/tests stop exercising them.

## ADR-009 — Groundhog is a sibling activity kind, not an `agent_loop` mode bit

**Status:** Accepted · 2026-04 · [T20260420-0510-2]

**Context.** Groundhog carries its own retry memory, checkpoint closure, and workspace snapshot semantics. Hiding that behind extra `agent_loop` flags would have buried a qualitatively different execution contract inside one overly broad type.

**Decision.** Represent Groundhog as its own `ActivityV2Spec::Groundhog` variant with a dedicated runner.

**Consequences.**
- Groundhog-specific state and docs have an obvious home.
- The agent loop type stays smaller and easier to reason about.
- Cost: ActivityV2 gains another sibling variant and the feature family becomes slightly broader.

## ADR-010 — Historical workflow inspection must not depend on live seeded job assets

**Status:** Superseded by ADR-014 · 2026-04 · [T20260423-0447]

**Context.** After [T20260419-2156], some older workflow assets such as duel no longer ship as runnable seeded jobs. Their historical run bundles and scoreboards can still exist on disk, and users still need to inspect that history.

**Decision.** Treat historical inspection as a stored-data concern, not a live-asset lookup. At the time, read-only surfaces such as `orbit run duel list` and latest `orbit run duel show` filtered persisted run bundles directly, and bare `orbit run duel` defaulted to the preserved scoreboard surface instead of the retired execution path.

**Consequences.**
- Retired workflows remained observable even after their executable assets disappeared.
- CLI retirement messaging and historical inspection behavior stayed aligned until the public run surface was simplified in [T20260425-2010].
- Cost: some read-only inspection paths no longer shared the same asset-validation gate as active workflow execution paths.

## ADR-011 — Merge object-valued job defaults with caller input, and surface early pipeline failures as synthetic job steps

**Status:** Accepted · 2026-04 · [T20260423-0445]

**Context.** The `task_auto_pipeline` / historical `orbit run ship` auto-dispatch path passes only a partial input object (`mode`, `base_branch`, `task_ids`, optional concurrency). The earlier job executor contract only applied `job.default_input` when the caller passed `null`, so any explicit object silently discarded required default keys like `max_tasks` and `max_bundle_size`. The same incident exposed a second operator gap: persisted v2 pipeline runs that failed before writing any concrete step files surfaced as `steps: []` with no `error_message` in workflow-specific show surfaces.

**Decision.** When both `job.default_input` and the caller input are JSON objects, Orbit performs a shallow merge and lets caller keys win on conflict; `null` and non-object caller inputs keep their pre-existing semantics. Separately, when a persisted v2 pipeline run fails and no recorded step already carries error detail, the pipeline worker writes a synthetic failed `JobRunStep` (`target_type: job`, `target_id: <job_id>`) so CLI/operator surfaces have a concrete message to show.

**Consequences.**
- Seeded workflows can rely on omitted keys inheriting from job defaults even when wrappers pass partial input objects.
- `orbit run ship --json`, `orbit run history`, and `orbit run show` surface actionable failure detail for early v2 pipeline failures instead of blank summaries.
- Cost: the job-level input contract is now a shallow merge rule that docs and tests must preserve, and run history can include synthetic job-level failure steps that were not literal authored YAML steps.

## ADR-012 — Direct v2 job runs are durable job runs, not audit-only executions

**Status:** Accepted · 2026-04 · [T20260423-2004-4]

**Context.** `orbit job run <schemaVersion: 2 yaml>` returned a run ID and wrote v2 audit JSONL, but did not create the persisted `JobRun` bundle that run inspection, the dashboard, and other operator surfaces read. That made the returned run ID less useful than IDs from pipeline-dispatched workflows and weakened Orbit's auditability story.

**Decision.** Treat direct v2 job execution as a normal durable job run at the public CLI/API boundary. The direct-run wrapper inserts a `JobRun`, marks it running, writes `state.json`, executes the same normalized v2 job path with the stored run ID, records a synthetic job-level step for the final pipeline/error surface, and finalizes the run. The lower-level `run_job_v2_from_yaml_with_run_id` remains available for already-persisted pipeline workers.

**Consequences.**
- A run ID returned by `orbit job run` is inspectable through `orbit run history -j <job_name>` and `orbit run show <run_id>`.
- Ad hoc YAML runs remain visible even when the job is not part of the live catalog, because history can fall back to stored run bundles.
- Cost: direct v2 execution now has persistence side effects and can record synthetic job-level steps that were not literal authored YAML steps.

## ADR-013 — Job catalog discovery honors layer precedence

**Status:** Accepted · 2026-04 · [T20260425-0204]

**Context.** Jobs are scoped as `MergeByKey`, but the v2 job catalog rejected a workspace/global duplicate `task_auto_pipeline` as invalid. That made normal workspace overrides fail before `orbit run ship` could start.

**Decision.** Load catalog directories from highest to lowest precedence and keep the first job for each `metadata.name`; environment job dirs outrank workspace jobs, and workspace jobs outrank global seeded jobs. Keep duplicate-name rejection inside a single directory tree so one layer remains internally unambiguous.

**Consequences.**
- Workspace workflows can override seeded defaults without deleting global resources.
- Public run aliases and pipeline-worker lookup paths share the same first-wins catalog resolution.
- Cost: lower-precedence job assets can be shadowed silently, so debugging an unexpected workflow now requires checking catalog source paths.

## ADR-014 — Public run workflows are execution aliases only

**Status:** Superseded by ADR-020 · 2026-04 · [T20260425-2010]

**Context.** `orbit run` had become a mixed surface: some entries executed workflows, while `ship list/show` and `duel list/show` browsed history that was already available through the then-generic job-run inspection surface. At the same time, the explicit task ship path and auto-dispatch path needed different job targets, and planning duel support had a live workflow alias without a seeded runnable job asset.

**Decision.** Treat `orbit run` as an execution-alias surface only. `orbit run ship <TASK_ID>...` dispatches explicit bundles through `task_pr_pipeline` or `task_local_pipeline` selected by `--mode`; `orbit run ship-auto` dispatches `task_auto_pipeline`; `orbit run duel-plan <TASK_ID>` dispatches the seeded `job_duel_plan_pipeline`; `orbit run job <JOB_ID>` remains the direct job surface. ADR-020 supersedes the inspection placement part of this decision.

**Consequences.**
- The public command grammar separates execution from inspection and removes workflow-specific history aliases.
- Explicit task shipping no longer routes through the auto-dispatch job, so task IDs map directly to the PR/local bundle jobs.
- Planning duels are runnable again through a seeded activity/job pair instead of a stale workflow alias.
- Cost: users of `orbit run ship local`, `orbit run ship list/show`, and `orbit run duel list/show` must update their command muscle memory and scripts.

## ADR-015 — CLI backend resolves executor args, not just provider commands

**Status:** Accepted · 2026-04 · [T20260423-0114]

**Context.** A local ship run for [T20260423-0114] failed in `backend: cli` before task execution because Orbit launched `codex --sandbox workspace-write` with piped stdin. The seeded Codex executor already declared `args: [exec, --json]`, but the v2 host boundary only returned a provider command string, so those static args were lost.

**Decision.** Make the v2 CLI host boundary return a resolved CLI executor (`command` plus static `args`) and have `cli_runner.rs` prepend those executor args before provider runtime args from the retained `AgentRuntime`. Environment command overrides remain command overrides; registered executor args still define the static CLI mode.

**Consequences.**
- `backend: cli` now honors seeded and workspace executor definitions for subprocess shape.
- Codex task runs enter non-interactive `codex exec --json` instead of the interactive TUI.
- Cost: the engine/core boundary is slightly wider than a single string and every smoke host implementing `V2RuntimeHost` must model executor args explicitly.

## ADR-016 — Activity catalogs honor layer precedence and activity execution stays job-owned

**Status:** Accepted · 2026-04 · [T20260426-0047]

**Context.** Activity assets are scoped as `MergeByKey`, but the catalog still rejected workspace/global duplicate names such as `pr_open`. The same product pass exposed that `orbit activity run` made activities look like a standalone public execution surface even though shipped workflows execute them through jobs.

**Decision.** Load activity catalog directories from highest to lowest precedence and keep the first activity for each `metadata.name`; environment activity dirs outrank workspace activities, and workspace activities outrank global seeded activities. Keep duplicate-name rejection inside a single directory tree, and remove the public `orbit activity run` subcommand so `orbit activity` remains a catalog surface.

**Consequences.**
- Workspace activity overrides can coexist with global defaults without breaking `orbit activity list --ops` or job target resolution.
- Public execution stays concentrated in `orbit job run` and workflow aliases under `orbit run`.
- Cost: lower-precedence activity assets can be shadowed silently, and direct ad hoc activity execution is no longer a documented CLI workflow.

## ADR-017 — V2 job metrics persist invocation traces beside audit

**Status:** Accepted · 2026-04 · [T20260426-0526]

**Context.** V2 job execution emitted rich audit JSONL, but `orbit metrics` reads agent and tool usage from the SQLite invocation store. After the v1 runner trace hook was removed, v2 CLI agent-loop runs could finish successfully while metrics reported no invocations or tool calls.

**Decision.** Treat invocation metrics as a first-class v2 job side effect rather than an audit scrape. `DispatchOutcome` may carry an `InvocationTrace`; the job executor persists that trace through `V2RuntimeHost` with the durable run ID, step ID, canonical agent/model identity, and task IDs from rendered input. The CLI backend derives the trace by parsing structured provider stdout, while HTTP loop paths convert `LoopOutcome` usage and tool-call names into the same store shape.

**Consequences.**
- `orbit metrics` can report v2 job agent usage and tool calls without depending on audit-log parsing.
- CLI and HTTP agent-loop paths converge on the same invocation-store contract.
- Cost: job execution now has another persistence side effect, and CLI metrics remain limited by the provider harness output format.

## ADR-018 — File-backed run traces live under workspace state

**Status:** Accepted · 2026-04 · [T20260426-0519]

**Context.** The v2 activity/job audit JSONL tree was written directly under `.orbit/audit/`, which made runtime traces a first-level sibling of durable authoring surfaces such as resources, tasks, and graph artifacts. That placement also obscured the distinction between the SQLite command audit store and file-backed run reconstruction artifacts.

**Decision.** Store activity/job audit JSONL and payload blobs under `.orbit/state/audit/`, with `v2_loop/`, `loop/`, and `blobs/` as siblings below that state root. Keep the SQLite command audit database at its existing persistence path and keep the v2 envelope's `workspace_path` field as the cross-workspace filter.

**Consequences.**
- Workspace runtime artifacts now live together under `.orbit/state/`.
- The file layout more clearly separates command audit queries from run-trace reconstruction files.
- Cost: existing local `.orbit/audit/` artifacts are legacy files; readers looking for historical runs may need to check both locations during any manual transition period.

## ADR-019 — Run inspection reads v2 traces through runtime accessors

**Status:** Accepted · 2026-04 · [T20260426-0705], [T20260426-0709]

**Context.** Operators need to inspect the v2 envelope tree from `orbit run`, but letting CLI command rendering parse `.orbit/state/audit/` paths directly couples user-facing inspection to engine-owned persistence details. Step selectors also drifted because durable v2 runs can store synthetic job-level steps while the envelope carries the activity DAG `step.id`.

**Decision.** Keep file-layout and blob-reading knowledge in orbit-core runtime accessors, and expose `orbit run events`, `orbit run trace`, and `orbit run logs` through those accessors. Treat envelope `step.started.step_id` as the primary user-facing step selector, with legacy `JobRunStep.target_id` and numeric step indexes as fallbacks.

**Consequences.**
- `orbit run events` and `orbit run trace` give operators chronological and tree-shaped views of run-local audit envelopes.
- Run-log stdout/stderr reading now follows the same boundary as v2 audit sink construction.
- Cost: the runtime layer now owns a read-side view model for audit JSONL, so envelope schema changes must update both writer and accessor tests together.

## ADR-020 — Run inspection belongs to `orbit run`

**Status:** Accepted · 2026-04 · [T20260426-0742]

**Context.** After run inspection gained history, state, logs, events, and trace commands, keeping duplicate job-level inspection aliases made `orbit job` mix catalog/execution responsibilities with run browsing. The help output also taught users two places to do the same inspection work.

**Decision.** Remove the public job-level history and run-state aliases. Keep `orbit job` focused on `list`, `show`, and direct `run`; use `orbit run history -j <JOB_ID>` and `orbit run show <RUN_ID>` for durable run inspection.

**Consequences.**
- Operators have one public command family for job-run inspection.
- The `orbit run` inspection commands keep their existing history/state behavior.
- Cost: scripts and muscle memory that used the removed aliases must migrate to the `orbit run` forms.

## ADR-021 — CLI subprocess output is a live tracing stream and a retained audit blob

**Status:** Accepted · 2026-04 · [T20260426-2313]

**Context.** The CLI backend captured subprocess stdout/stderr as bulk buffers and surfaced them only after process exit through blob refs. That preserved auditability, but it left dashboard/log-feed work without a live structured signal for agent progress.

**Decision.** Read CLI subprocess stdout and stderr line by line in the existing pipe-reader threads, append each raw line to the retained byte buffer, and emit one `tracing::info!` event per line with `provider`, `stream`, `job_run_id`, `task_id`, and redacted `line`. Keep `CliInvocationFinished` and its stdout/stderr blob refs on the same captured-byte path as before.

**Consequences.**
- Future tracing sinks can build a merged live feed without scraping subprocess blobs.
- The audit/blob contract, exit-code handling, and timeout handling remain the durable completion record.
- Cost: CLI output now has two observability paths; the tracing line text is UTF-8/lossy and newline-stripped while the retained blob bytes remain the archival source.

---

## Task References

- **[T20260418-2018]** — Add `JobV2` DAG constructs (`parallel`, `fan_out`, `loop`, `retry`, `when`).
- **[T20260418-2019]** — Add v2 activity name resolution and pipeline skeleton assets.
- **[T20260418-2143]** — Wire `V2RuntimeHost` in orbit-core and add `orbit activity run-v2`.
- **[T20260418-2210]** — Reshape `V2RuntimeHost` to keep `orbit-agent` types out of orbit-core.
- **[T20260419-0002]** — Add `workspace_path` provenance to the v2 audit envelope.
- **[T20260419-0104]** — Add `backend: cli` dispatch for v2 `agent_loop`.
- **[T20260419-0622-3]** — Add `task_gate_pipeline`.
- **[T20260419-0623]** — Add `task_auto_pipeline`.
- **[T20260419-0623-2]** — Add `task_epic_pipeline`.
- **[T20260419-2014]** — Merge `orbit-types` into `orbit-common`.
- **[T20260419-2156]** — Retire v1 assets and drop the transitional v2 naming.
- **[T20260419-2347]** — Seed activities and workflows on `orbit init`.
- **[T20260420-0510-2]** — Add the Groundhog v1 activity runner.
- **[T20260423-0114]** — Expose the `backend: cli` executor-args gap during a local task ship run.
- **[T20260423-0445]** — Merge object-valued job defaults over explicit run input and persist synthetic failed job steps for early v2 pipeline failures.
- **[T20260423-0447]** — Restore usable `orbit run duel` read-only surfaces after duel workflow retirement.
- **[T20260423-2004-4]** — Persist direct v2 `orbit job run` executions into durable job-run records and state.
- **[T20260425-0204]** — Make v2 job catalog discovery honor workspace-over-global `MergeByKey` precedence.
- **[T20260425-2010]** — Refactor `orbit run` task workflow commands and revive `duel-plan` as a seeded run workflow.
- **[T20260426-0047]** — Make v2 activity catalog discovery honor workspace-over-global `MergeByKey` precedence and remove the public `orbit activity run` command.
- **[T20260426-0526]** — Restore v2 job invocation trace persistence so `orbit metrics` can report agent and tool usage.
- **[T20260426-0519]** — Move file-backed activity/job audit traces under `.orbit/state/audit`.
- **[T20260426-0705]** — Expose v2 run audit events through `orbit run events` and `orbit run trace`.
- **[T20260426-0709]** — Align run step selectors on activity `step.id` and move CLI invocation log reading behind orbit-core runtime accessors.
- **[T20260426-0742]** — Remove duplicate job-level run inspection aliases and keep run inspection under `orbit run`.
- **[T20260426-2313]** — Stream CLI subprocess stdout/stderr through structured tracing events while retaining the existing audit/blob path.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
