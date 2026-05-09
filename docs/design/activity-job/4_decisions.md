# Activity / Job — Decisions

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-09 (T20260427-34, T20260427-36, T20260427-38, T20260427-40, T20260508-3, T20260508-8, T20260509-2)

This ADR log records the decisions that define the current Activity / Job substrate. Entries are append-only and stay in place when later ADRs supersede or fold them. See [1_overview.md](./1_overview.md) for the feature summary, [2_design.md](./2_design.md) for the current implementation, and [3_vision.md](./3_vision.md) for the questions that may force more decisions.

The log now keeps four load-bearing rollup ADR bodies. Folded entries remain at their original numbers with `Status: Superseded by ADR-NNN (folded)` and a one-line pointer, per [CONVENTIONS §4a](../CONVENTIONS.md#4a-rollup-adrs).

---

## ADR-001 — Canonical v2 assets normalize into one execution contract

**Status:** Accepted · 2026-05 · [T20260419-2156], [T20260418-2143], [T20260419-0104], [T20260418-2019], [T20260423-0445], [T20260425-0204], [T20260419-2347], [T20260426-0047], [T20260428-8], [T20260506-18]

**Context.** Activity/job correctness depends on making authoring conveniences disappear before execution. The old log carried separate ADRs for schema retirement, backend resolution, target refs, defaults, catalog precedence, seeded assets, and workflow admission, but all enforce the same boundary: YAML is human-authored input, while execution sees normalized, validated runtime state.

**Decision.** Treat `schemaVersion: 2` as the only activity/job asset family, load seeded and workspace catalogs with explicit layer precedence, resolve authoring sugar (`target: activity:<name>`, `backend: auto`, object-valued defaults, and workflow admission) before dispatch, and keep seeded activities/jobs as executable reference contracts for that normalized surface. Direct task-workflow admission remains a workflow-specific normalization path rather than a generic task-update rule.

Folded instances:

| ADR | Instance folded into this rollup |
|-----|----------------------------------|
| ADR-003 | `backend: auto` resolves once before dispatch. |
| ADR-004 | `target: activity:<name>` is authoring sugar resolved before execution. |
| ADR-008 | Seeded activities and jobs are load-bearing runtime contracts. |
| ADR-011 | Object-valued job defaults shallow-merge with caller input, and early failures get synthetic job steps. |
| ADR-013 | Job catalog discovery honors layer precedence. |
| ADR-016 | Activity catalog discovery honors layer precedence, and activity execution stays job-owned. |
| ADR-026 | Workflow admission is distinct from generic task updates. |

**Consequences.**
- The runtime now documents and validates one typed activity/job surface.
- Human-authored YAML stays readable while executors consume concrete steps, concrete backends, merged inputs, and first-wins catalog entries.
- New workspaces start with real executable reference assets rather than empty examples.
- Costs retained from folded entries:
- Cost: old assets stop limping along; migration work becomes mandatory instead of gradual.
- Cost: callers must remember to run the normalization pass before dispatch, and any missed call site fails as a structural bug.
- Cost: the load path owns more normalization logic, and stale refs fail before dispatch instead of being lazily recoverable.
- Cost: seeded assets become part of the public maintenance burden and can drift if docs/tests stop exercising them.
- Cost: the job-level input contract is now a shallow merge rule that docs and tests must preserve, and run history can include synthetic job-level failure steps that were not literal authored YAML steps.
- Cost: lower-precedence job assets can be shadowed silently, so debugging an unexpected workflow now requires checking catalog source paths.
- Cost: lower-precedence activity assets can be shadowed silently, and direct ad hoc activity execution is no longer a documented CLI workflow.
- Cost: task lifecycle semantics are no longer uniform across all status mutation surfaces; reviewers must distinguish workflow admission from ordinary task updates.

## ADR-002 — Host boundaries and agent dispatch stay explicit

**Status:** Accepted · 2026-05 · [T20260419-2014], [T20260418-2210], [T20260419-0104], [T20260423-0114], [T20260427-48], [T20260430-15], [T20260418-2018], [T20260419-0623-2], [T20260420-0510-2], [T20260428-9], [T20260428-12], [T20260506-16], [T20260506-17], [T20260505-22], [T20260506-18]

**Context.** The agent-loop path is where activity/job can most easily leak provider implementation details, mutable sessions, or role configuration across crate boundaries. The split ADRs all defended the same shape: shared types live low, orbit-core hosts primitive services, the engine dispatches concrete activity specs, and provider/backends remain explicit choices.

**Decision.** Keep activity/job types in `orbit-common`, keep orbit-core free of `orbit-agent` transport types, and route `backend: cli` through retained provider runtimes behind a host-resolved executor contract. Scope stateful agent features narrowly: loop `session:` is HTTP-only, Groundhog is its own activity kind, role config from `[agent.<role>]` overrides inline settings field-by-field, task-aware CLI envelopes carry durable run context, and provider static-arg fixups run before sandbox dispatch.

Folded instances:

| ADR | Instance folded into this rollup |
|-----|----------------------------------|
| ADR-005 | Cross-iteration `session:` binding is loop-scoped and HTTP-only. |
| ADR-006 | Retained CLI runtimes implement `backend: cli`. |
| ADR-009 | Groundhog is a sibling activity kind, not an `agent_loop` mode bit. |
| ADR-015 | CLI backend resolves executor args, not just provider commands. |
| ADR-025 | Codex CLI dynamic flags stay in provider runtime config. |
| ADR-027 | `orbit init` writes per-role agent settings. |
| ADR-031 | `[agent.<role>]` config overrides inline `agent_loop` settings at dispatch. |
| ADR-032 | CLI agent envelopes carry durable task and run context. |
| ADR-040 | Provider static-arg fixups apply before sandbox dispatch. |
| ADR-041 | `orbit init` uses a recommendation-first setup wizard. |

**Consequences.**
- Parsing, validation, dispatch, and CLI display share one Rust type family without making orbit-core depend on provider transport objects.
- CLI and HTTP agent-loop paths remain intentionally different where their capabilities differ, especially around sessions and tool enforcement.
- First-run and per-role agent choices live in user config while YAML stays reusable across workspaces.
- Costs retained from folded entries:
- Cost: `orbit-common` now owns a wider slice of runtime vocabulary and has to stay disciplined about not accreting behavior.
- Cost: session reuse becomes a narrowly scoped feature instead of a general-purpose memory layer.
- Cost: the feature now has materially different semantics between HTTP and CLI, especially around tool enforcement.
- Cost: ActivityV2 gains another sibling variant and the feature family becomes slightly broader.
- Cost: the engine/core boundary is slightly wider than a single string and every smoke host implementing `V2RuntimeHost` must model executor args explicitly.
- Cost: the v2 host boundary exposes a provider-config map, so backend CLI dispatch remains aware of provider-specific runtime settings.
- Cost: until [T20260428-12] landed, the values written to `config.toml` were inert — they round-tripped but did not influence dispatch, so reviewers had to treat the behavior as half-shipped during that window.
- Cost: dispatch now has one more clone-and-mutate path per role-tagged step. The same role might get queried multiple times within one job run; if that ever shows up in profiles, memoize at the executor level rather than in the host trait.
- Cost: the `V2RuntimeHost` seam now has a method that is purely a config-config concern. Tests that build their own mock host get a free `None` default, but a host that wants to exercise the override path has to opt in explicitly.
- Cost: CLI stdin blobs now contain more task prose, so audit blob readers should continue treating those blobs as diagnostic artifacts rather than small control messages.
- Cost: provider static-arg fixups mean executor YAML values such as Claude's `--debug-file` path are no longer honored verbatim; maintainers must read dispatcher behavior alongside assets.
- Cost: prompt collection now owns display formatting and a small choice loop, so tests must cover interaction flow in addition to config values.

## ADR-003 — Resolve `backend: auto` once, before dispatch

**Status:** Superseded by ADR-001 (folded) · 2026-04 · [T20260418-2143], [T20260419-0104]

Folded into ADR-001's rollup for canonical v2 asset normalization.

## ADR-004 — `target: activity:<name>` is authoring sugar, not an execution primitive

**Status:** Superseded by ADR-001 (folded) · 2026-04 · [T20260418-2019]

Folded into ADR-001's rollup for canonical v2 asset normalization.

## ADR-005 — Cross-iteration `session:` binding is a loop-scoped HTTP-only feature

**Status:** Superseded by ADR-002 (folded) · 2026-04 · [T20260418-2018], [T20260419-0104], [T20260419-0623-2]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-006 — Keep the retained CLI runtimes as the implementation of `backend: cli`

**Status:** Superseded by ADR-002 (folded) · 2026-04 · [T20260419-0104], [T20260418-2210]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-007 — Run state, audit, and operator inspection are durable layers

**Status:** Accepted · 2026-05 · [T20260419-0002], [T20260423-0447], [T20260423-2004-4], [T20260426-0526], [T20260426-0519], [T20260426-0705], [T20260426-0709], [T20260425-2010], [T20260426-0742], [T20260426-2313], [T20260426-2349], [T20260430-31], [T20260505-8], [T20260506-18]

**Context.** Activity/job execution produces operator evidence at several layers: audit envelopes, job-run records, metrics, live traces, retained blobs, run-inspection commands, PR handoff summaries, and cancellation state. The separate ADRs all instantiate the same rule: runtime output is durable workflow state, not process stdout or live assets pretending to be history.

**Decision.** Keep a v2 audit envelope layered over lower-level loop audit, persist direct and pipeline job runs as durable `JobRun` bundles, store file-backed traces under workspace state, read run inspection through runtime accessors, and place public run browsing under `orbit run`. CLI subprocess output may stream through tracing, but retained blobs remain archival; redaction belongs to the tracing subscriber; metrics, execution summaries, and cancellation are persisted as first-class run/task state.

Folded instances:

| ADR | Instance folded into this rollup |
|-----|----------------------------------|
| ADR-010 | Historical workflow inspection reads stored data, not live seeded assets. |
| ADR-012 | Direct v2 job runs persist durable job-run bundles. |
| ADR-017 | V2 job metrics persist invocation traces beside audit. |
| ADR-018 | File-backed run traces live under workspace state. |
| ADR-019 | Run inspection reads v2 traces through runtime accessors. |
| ADR-020 | Run inspection belongs to `orbit run`. |
| ADR-021 | CLI subprocess output is both a live tracing stream and retained audit blob. |
| ADR-022 | CLI output redaction belongs to the tracing subscriber. |
| ADR-036 | Task PRs require durable execution summaries. |
| ADR-038 | Dashboard cancellation is a durable job-run transition. |

**Consequences.**
- Reviewers can traverse runs by job, step, activity, and raw loop detail without parsing agent process output as workflow handoff.
- Operator surfaces share durable state for history, metrics, logs, cancellation, and PR handoff.
- The file layout clearly separates command audit queries from run-trace reconstruction files.
- Costs retained from folded entries:
- Cost: audit review now spans two related storage layouts instead of one.
- Cost: some read-only inspection paths no longer shared the same asset-validation gate as active workflow execution paths.
- Cost: direct v2 execution now has persistence side effects and can record synthetic job-level steps that were not literal authored YAML steps.
- Cost: job execution now has another persistence side effect, and CLI metrics remain limited by the provider harness output format.
- Cost: existing local `.orbit/audit/` artifacts are legacy files; readers looking for historical runs may need to check both locations during any manual transition period.
- Cost: the runtime layer now owns a read-side view model for audit JSONL, so envelope schema changes must update both writer and accessor tests together.
- Cost: scripts and muscle memory that used the removed aliases must migrate to the `orbit run` forms.
- Cost: CLI output now has two observability paths; the tracing line text is UTF-8/lossy and newline-stripped while the retained blob bytes remain the archival source.
- Cost: tests that inspect tracing safety must capture formatted subscriber output, not raw `Event` fields.
- Cost: manual or custom-body shipment paths must still persist task summaries before opening the PR, even when the caller already prepared a complete body.
- Cost: direct in-process job runs still cannot safely self-signal; dashboard cancellation is primarily the durable pipeline-worker/operator path.

## ADR-008 — Seed reference activities and jobs as load-bearing runtime contracts

**Status:** Superseded by ADR-001 (folded) · 2026-04 · [T20260419-2347], [T20260419-0622-3], [T20260419-0623], [T20260419-0623-2]

Folded into ADR-001's rollup for canonical v2 asset normalization.

## ADR-009 — Groundhog is a sibling activity kind, not an `agent_loop` mode bit

**Status:** Superseded by ADR-002 (folded) · 2026-04 · [T20260420-0510-2]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-010 — Historical workflow inspection must not depend on live seeded job assets

**Status:** Superseded by ADR-007 (folded) · 2026-04 · [T20260423-0447]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-011 — Merge object-valued job defaults with caller input, and surface early pipeline failures as synthetic job steps

**Status:** Superseded by ADR-001 (folded) · 2026-04 · [T20260423-0445]

Folded into ADR-001's rollup for canonical v2 asset normalization.

## ADR-012 — Direct v2 job runs are durable job runs, not audit-only executions

**Status:** Superseded by ADR-007 (folded) · 2026-04 · [T20260423-2004-4]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-013 — Job catalog discovery honors layer precedence

**Status:** Superseded by ADR-001 (folded) · 2026-04 · [T20260425-0204]

Folded into ADR-001's rollup for canonical v2 asset normalization.

## ADR-014 — Public run workflows are execution aliases only

**Status:** Superseded by ADR-023 (folded) · 2026-04 · [T20260425-2010]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-015 — CLI backend resolves executor args, not just provider commands

**Status:** Superseded by ADR-002 (folded) · 2026-04 · [T20260423-0114]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-016 — Activity catalogs honor layer precedence and activity execution stays job-owned

**Status:** Superseded by ADR-001 (folded) · 2026-04 · [T20260426-0047]

Folded into ADR-001's rollup for canonical v2 asset normalization.

## ADR-017 — V2 job metrics persist invocation traces beside audit

**Status:** Superseded by ADR-007 (folded) · 2026-04 · [T20260426-0526]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-018 — File-backed run traces live under workspace state

**Status:** Superseded by ADR-007 (folded) · 2026-04 · [T20260426-0519]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-019 — Run inspection reads v2 traces through runtime accessors

**Status:** Superseded by ADR-007 (folded) · 2026-04 · [T20260426-0705], [T20260426-0709]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-020 — Run inspection belongs to `orbit run`

**Status:** Superseded by ADR-007 (folded) · 2026-04 · [T20260426-0742]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-021 — CLI subprocess output is a live tracing stream and a retained audit blob

**Status:** Superseded by ADR-007 (folded) · 2026-04 · [T20260426-2313]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-022 — CLI output redaction belongs to the tracing subscriber

**Status:** Superseded by ADR-007 (folded) · 2026-04 · [T20260426-2349]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-023 — Seeded task-shipment workflows are deterministic, recoverable, and lock-aware

**Status:** Accepted · 2026-05 · [T20260427-33], [T20260425-2010], [T20260427-45], [T20260430-9], [T20260430-12], [T20260430-14], [T20260421-0542-2], [T20260430-27], [T20260430-30], [T20260430-26], [T20260427-34], [T20260427-36], [T20260505-2], [T20260505-10], [T20260506-18]

**Context.** The seeded task workflows added many small ADRs as shipment behavior grew: run aliases, deterministic auto-dispatch, remote base selection, recovery hooks, backlog exclusions, operator status, friction admission, and lock cleanup. They are one decision family: task shipment is an explicit durable workflow, not an advisory agent step or hidden side effect.

**Decision.** Keep `orbit run` workflow aliases focused on execution, make automatic task shipment deterministic from backlog listing through gate fan-out, default shipping worktrees to fetched remote base refs, admit tasks through status-aware workflow gates, and protect overlapping work with durable task-lock reservations whose seeded TTL covers the child wait budget. Recovery is bounded and step-scoped on direct shipment workflows, child pipeline joins are followed by deterministic success guards after required cleanup, operator status is derived from persisted pipeline state, accepted friction reports enter auto-backlog by `status: backlog`, and run-owned reservations clean up when their owner run reaches a terminal state.

Folded instances:

| ADR | Instance folded into this rollup |
|-----|----------------------------------|
| ADR-014 | Public run workflows are execution aliases only. |
| ADR-024 | Shipping worktrees default to fetched remote base refs. |
| ADR-028 | Job-level recovery handles retry-exhausted step errors. |
| ADR-029 | The first direct-shipment recovery default was deterministic and conservative. |
| ADR-030 | Default recovery is step-scoped and agent-driven. |
| ADR-033 | Auto-backlog lock exclusions are structured output. |
| ADR-034 | `ship-auto` reports operator workflow status from durable pipeline state. |
| ADR-035 | Gate reservations release after terminal child waits. |
| ADR-037 | Accepted friction reports enter auto-backlog by status. |
| ADR-039 | Run-owned task-lock reservations clean up at owner terminal. |

**Consequences.**
- Task shipment workflows expose durable admission, recovery, status, and lock state without asking downstream steps to parse model output.
- Auto-dispatch no longer depends on provider credentials before it has deterministic backlog bundles.
- Gate-owned reservations serialize overlapping bundles while their owner run is alive and are released by both seeded early-release steps and engine-owned terminal cleanup.
- Seeded gate defaults require `ttl_seconds >= dispatch_timeout_seconds` so a legal child shipment wait cannot outlive its admission reservation.
- Costs retained from folded entries:
- Cost: the auto-dispatch audit trail no longer contains a model-authored advisory grouping note.
- Cost: users of `orbit run ship local`, `orbit run ship list/show`, and `orbit run duel list/show` must update their command muscle memory and scripts.
- Cost: default shipping workflows now require the configured base branch to be fetchable from `origin`; callers that intentionally operate without a remote must opt into `base_sync: local`.
- Cost: job authors must make the recovery activity generic enough for every retryable step in that job.
- Cost: this is intentionally conservative; it does not perform semantic git cleanup, task mutation, or child-run reconciliation until a more specific recovery policy is justified.
- Cost: default recovery now depends on a CLI agent runtime being available, and authors must decide which steps deserve recovery rather than flipping one workflow-level switch.
- Cost: the Rust serializer and seeded activity YAML schema now duplicate the exclusion shape and must be kept in sync.
- Cost: the CLI formatter now knows selected fields from `task_auto_pipeline` state, so future pipeline key renames must either preserve compatibility or update the operator summary parser.
- Cost: `task_gate_pipeline` now relies on the dynamic `task_{{ input.mode }}_pipeline` job-name convention, so future gate modes must either follow that naming convention or refactor the dispatch selector.
- Cost: child dispatch status remains data until explicit guard steps run, so seeded workflow authors must preserve guard placement after cleanup when they fork task-shipment YAML.
- Cost: longer default gate reservations can block overlapping work for up to two hours if both explicit release and run-owned cleanup fail.
- Cost: reviewers must read friction eligibility as a status rule, not a task-type rule.
- Cost: job-run finalization and reservation reserve paths are more coupled, so new terminal run paths must route through the cleanup helper rather than writing directly to the job-run store.

## ADR-024 — Shipping worktrees default to fetched remote base refs

**Status:** Superseded by ADR-023 (folded) · 2026-04 · [T20260427-45]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-025 — Codex CLI dynamic flags stay in provider runtime config

**Status:** Superseded by ADR-002 (folded) · 2026-04 · [T20260427-48]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-026 — Workflow admission is distinct from generic task updates

**Status:** Superseded by ADR-001 (folded) · 2026-04 · [T20260428-8]

Folded into ADR-001's rollup for canonical v2 asset normalization.

## ADR-027 — `orbit init` is the writer for per-role agent settings

**Status:** Superseded by ADR-002 (folded) · 2026-04 · [T20260428-9]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-028 — Job-level recovery activity handles retry-exhausted step errors

**Status:** Superseded by ADR-023 (folded) · 2026-04 · [T20260430-9]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-029 — Ship default task-step recovery only on direct shipment workflows

**Status:** Superseded by ADR-023 (folded) · 2026-04 · [T20260430-12]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-030 — Default recovery is step-scoped and agent-driven

**Status:** Superseded by ADR-023 (folded) · 2026-04 · [T20260430-14]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-031 — `[agent.<role>]` config overrides inline `agent_loop` settings at dispatch

**Status:** Superseded by ADR-002 (folded) · 2026-04 · [T20260428-12]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-032 — CLI agent envelopes carry durable task and run context

**Status:** Superseded by ADR-002 (folded) · 2026-04 · [T20260430-15]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-033 — Auto-backlog lock exclusions are structured output

**Status:** Superseded by ADR-023 (folded) · 2026-04 · [T20260421-0542-2]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-034 — `ship-auto` reports operator workflow status from durable pipeline state

**Status:** Superseded by ADR-023 (folded) · 2026-04 · [T20260430-27], [T20260430-30]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-035 — Gate reservations release after terminal child waits

**Status:** Superseded by ADR-023 (folded) · 2026-04 · [T20260430-26]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-036 — Task PRs require durable execution summaries

**Status:** Superseded by ADR-007 (folded) · 2026-05 · [T20260430-31]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-037 — Accepted friction reports enter auto-backlog by status

**Status:** Superseded by ADR-023 (folded) · 2026-05 · [T20260505-2]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-038 — Dashboard cancellation is a durable job-run transition

**Status:** Superseded by ADR-007 (folded) · 2026-05 · [T20260505-8]

Folded into ADR-007's rollup for durable run state and operator inspection.

## ADR-039 — Run-owned task-lock reservations clean up at owner terminal

**Status:** Superseded by ADR-023 (folded) · 2026-05 · [T20260505-10]

Folded into ADR-023's rollup for seeded task-shipment workflow automation.

## ADR-040 — Provider static-arg fixups apply before sandbox dispatch

**Status:** Superseded by ADR-002 (folded) · 2026-05 · [T20260505-22]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-041 — `orbit init` uses a recommendation-first setup wizard

**Status:** Superseded by ADR-002 (folded) · 2026-05 · [T20260506-16], [T20260506-17]

Folded into ADR-002's rollup for explicit agent dispatch boundaries.

## ADR-042 — One-task PR bodies start with the task contract

**Status:** Accepted · 2026-05 · [T20260508-3]

**Context.** Task-shipping PRs now carry one task, but the default generated body still reflected the older batch shape. Reviewers had to leave the PR to read the task description and acceptance criteria, while GitHub already rendered the changed-file list natively.

**Decision.** Render one-task PR bodies as `## Task`, optional collapsed `## Execution Summary`, `## Validation`, and `## Branch Freshness`. The task section includes the task link, verbatim description, and plain-bullet acceptance criteria. Multi-task callers keep the legacy body while those paths remain supported.

## ADR-043 — Epic review status is a shipped stop state

**Status:** Accepted · 2026-05 · [T20260427-38]

**Context.** `task_epic_pipeline` exits from deterministic `load_epic` snapshots, while normal child shipment workflows stop successful subtasks in `review` for human handoff. Treating `review` as open work made a clean epic cycle redispatch already-shipped subtasks or run until its iteration ceiling.

**Decision.** For epic orchestration only, treat `review` as a shipped stop state: `load_epic` omits review subtasks from the open workset, allows them to satisfy `all_terminal`, and maps their epic summary state to `done` while preserving the raw task status.

**Consequences.**
- Epic loops can converge after normal PR/local child shipment without embedding human approval into the pipeline.
- Operators can still inspect raw `status: "review"` in the final snapshot and task records before approving lifecycle completion.
- Cost: `summarize_epic`'s `done` counter now includes review-shipped subtasks for epic completion, so readers must distinguish pipeline completion from task approval.

## ADR-044 — Epic orchestrator does not block on child runs

**Status:** Accepted · 2026-05 · [T20260427-40]

**Context.** The `epic_orchestrator` activity exists to make one judgment cycle: read the deterministic epic snapshot, choose ready bundles, and dispatch child `task_gate_pipeline` runs. Its previous instruction also made the HTTP agent call `orbit.pipeline.wait`, but a normal gate-and-ship envelope can exceed the orchestrator's wall-clock by hours: gate admission can wait, child dispatch can wait, and implementer activities have their own long timeout.

**Decision.** Keep the orchestrator fire-and-forget. It may call `orbit.pipeline.invoke`, then must return structured `dispatched_run_ids`. `task_epic_pipeline` performs the blocking join through deterministic `pipeline_wait`, then runs `refresh_epic` so loop exit still keys off durable task state. The per-cycle wait budget should satisfy `iteration_wait_seconds >= task_gate_pipeline.max_wait_seconds + task_gate_pipeline.dispatch_timeout_seconds` for full-envelope joins; seeded defaults currently keep `iteration_wait_seconds` at the pipeline wait cap of 7200 seconds, below the theoretical 10800-second gate envelope, so a timeout can surface a still-running child.

**Consequences.**
- A premium HTTP orchestrator session is bounded to a dispatch decision cycle instead of babysitting child workflow polling.
- Audit lineage moves from agent tool calls to deterministic `ActivityStarted` / `ActivityFinished` envelopes for the join step; the child relationship remains reconstructable from `dispatched_run_ids` and run-step state.
- If `pipeline_wait` times out while a child is still running, the next deterministic `load_epic` snapshot still shows open work. Redundant redispatch is bounded by the gate pipeline's task-lock reservation: overlapping context files are denied while the child reservation is active, and TTL remains the abandoned-run fallback.

## ADR-045 — CLI subprocess cwd is runtime-owned workspace state

**Status:** Accepted · 2026-05 · [T20260508-8]

**Context.** Per-run worktrees are supposed to isolate task implementation, but `backend: cli` children previously inherited the pipeline worker cwd and only learned the intended workspace through prompt/input data.

**Decision.** Resolve CLI subprocess cwd before spawn from `input.workspace_path`, then task snapshot `workspace_path`, then best-effort `ToolContext.workspace_root`. Declared input/task paths fail fast if stale, and the selected cwd is recorded in the CLI started audit event plus line-level tracing.

**Consequences.**
- The runtime, not the prompt, controls where relative paths in provider CLIs resolve.
- Groundhog and CLI dispatch share one workspace resolver, reducing future drift between orchestration and implementation attempts.
- Cost: stale declared worktrees now fail before spawn instead of silently running from the parent process directory.

## ADR-046 — Job executor internals split by execution responsibility

**Status:** Accepted · 2026-05 · [T20260509-2]

**Context.** The v2 job executor concentrated step dispatch, retry/recovery, construct orchestration, template rendering, validation, audit projection, and inline tests in one 2.8k-line file.

**Decision.** Keep the public job-executor API stable, but organize the implementation as `job_executor/` child modules with `mod.rs` holding the exported entrypoints and private helpers shared through module-scoped visibility.

**Consequences.**
- Reviewers can inspect retry/recovery, target dispatch, fan-out, loop, validation, and audit behavior in smaller files without changing runtime semantics.
- The split preserves the existing engine/core and CLI-runner boundaries; no new crate edge or provider type crosses the activity/job layer.
- Cost: private helper movement now requires maintaining intra-module visibility and imports across several files instead of one lexical scope.

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
- **[T20260421-0542-2]** — Add structured `list_backlog_tasks` output for context-lock exclusions.
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
- **[T20260426-2349]** — Move CLI tracing output redaction from `cli_runner` call sites into the default tracing formatter layer.
- **[T20260427-33]** — Remove the audit-only `dispatch_agent` step from `task_auto_pipeline`.
- **[T20260427-34]** — Add seeded pipeline success guards so non-succeeded child runs fail parent shipment workflows.
- **[T20260427-36]** — Align task-gate reservation TTL with the child dispatch wait budget.
- **[T20260427-38]** — Treat review as a shipped stop state for epic automation.
- **[T20260427-40]** — Move epic child-run waiting out of the orchestrator agent and into a deterministic workflow step.
- **[T20260427-45]** — Use freshly fetched remote base refs for default task-shipping worktrees.
- **[T20260427-48]** — Thread provider config into the v2 CLI backend and keep Codex dynamic flags exec-compatible.
- **[T20260428-8]** — Add workflow-specific task admission for task-starting workflows.
- **[T20260428-9]** — `orbit init` writes per-role agent settings to `[agent.<role>]` in `config.toml`.
- **[T20260428-12]** — Wire `[agent.<role>]` config into `agent_loop` dispatch via the `role:` field and a host-backed resolver.
- **[T20260430-9]** — Add a job-level recovery activity hook for retry-exhausted v2 step failures.
- **[T20260430-12]** — Ship a generic deterministic recovery activity for direct task shipment workflows.
- **[T20260430-14]** — Make default step recovery agent-driven and step-scoped.
- **[T20260430-15]** — Embed task-aware input and run context in backend: cli agent envelopes.
- **[T20260430-19]** — Shorten the Activity / Job design docs while preserving required structure.
- **[T20260430-26]** — Release task-gate reservations after terminal child shipment runs and expose active reservations through the lock view.
- **[T20260430-27]** — Make `ship-auto` output distinguish empty backlog, gated no-op, and waiting gate children.
- **[T20260430-30]** — Make `ship-auto` default text output human-readable while preserving JSON fields.
- **[T20260430-31]** — Require populated execution summaries before opening task PRs.
- **[T20260505-2]** — Admit accepted backlog friction reports in automatic backlog listing.
- **[T20260505-8]** — Add dashboard/runtime controls to cancel active job runs.
- **[T20260505-10]** — Release run-owned task lock reservations through engine-owned terminal cleanup and reserve-pressure reconciliation.
- **[T20260505-22]** — Rewrite Claude's `--debug-file` static arg at dispatch time so the log lands at a sandbox-allowed absolute path.
- **[T20260506-16]** — Replace raw `orbit init` agent prompts with a recommendation-first setup wizard.
- **[T20260506-17]** — Make `orbit init` recommend Codex for reviewer and implementer when available.
- **[T20260506-18]** — Compact activity-job ADRs via rollups.
- **[T20260508-3]** — Revise generated task PR bodies around the one-task-per-PR workflow.
- **[T20260508-8]** — Resolve backend: cli subprocess cwd from workspace context and record it in audit/tracing.
- **[T20260509-2]** — Split the v2 job executor into responsibility-focused modules without changing runtime behavior.

> Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
