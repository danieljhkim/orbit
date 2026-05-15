# Changelog

## 0.5.4

### Features

- **Project-learnings push-injection (L1/L2/L3)**: relevant learning summaries are now injected into agent context at three layers — engine pre-prompt before runtime spawn, MCP sidecar on path-bearing tools (`orbit.graph.show`, `orbit.graph.refs`, `orbit.task.show`), and a Claude Code `PreToolUse` hook on `Edit | Write | Read`. Summary-only payloads with per-session dedup, per-call caps, and an `ORBIT_SESSION_ID` envelope for cross-process dedup. ([ORB-00009])
- **First-class design-docs surface (`orbit.design.*` + `orbit design check` CLI)**: four MCP tools (`init`, `list`, `show`, `check`) plus a Rust port of the design-doc decay checker, with `orbit workspace init --design` seeding `docs/design/CONVENTIONS.md` when absent. `make check-design-docs` and `scripts/check_design_doc_decay.py` now wrap the Rust path. ([ORB-00019])
- **Default-seeded skills aligned across asset, registry, plugin, and router catalogs**: `orbit-learning` and `orbit-design` onboarded; `orbit-review-task` and `orbit-semantic` brought into the default seed and the plugin's `skills/` symlinks; three drift-detection unit tests guard the four catalogs against recurrence. Default seed bumped 7 → 11. ([ORB-00020], [ORB-00022])
- **`orbit workspace init --inject-agent-rules`**: opt-in flag writes an idempotent Orbit-rules block into `CLAUDE.md` and `AGENTS.md` at the workspace root, delimited by `<!-- orbit-managed:start/end -->` markers. Block content sourced from an editable asset; malformed marker pairs refuse to write. ([ORB-00023])
- **Inline task status transitions in the dashboard**: per-task actions row gains a status selector wired through the existing `PATCH /tasks/:id` backend, ordered by `STATUS_ORDER` with `done` last and excluding `rejected`/`archived`/`friction`. Surfaces a "no longer shown in dashboard list" notice when transitioning to `done`. ([ORB-00025])
- **`orbit.learning.*` exposed over MCP**: the full eight-tool learning surface (`add`, `list`, `search`, `show`, `update`, `supersede`, `prune`, `reindex`) is now reachable from every MCP client, not just `orbit tool run`. Restores parity with the `orbit-learning` skill instructions. ([ORB-00039])

### Fixes

- **PID identity & stale-run probe stability**: versioned `ps-lstart-utc-v1:` token replaces raw `lstart` output (recorded under `TZ=UTC`/`LC_ALL=C`) so live workers are no longer falsely marked failed across timezone changes; a new `OwnerIdentity::ProbeUnavailable` outcome distinguishes transient `ps` failures from genuinely dead PIDs, preventing single-probe terminalization of live workers. Adds arbiter-side `orbit.duel.plan.winner` regression coverage. Backward-compat read path (`LegacyLiveUnverified`) handles existing persisted tokens. ([ORB-00036], [ORB-00037])
- **Job-step error messages reach the dashboard**: `V2AuditEventKind::{StepFinished, RunFinished}` gain an optional `error_message` field; the failure reason is preserved at emit time and surfaced by the audit reader so the Steps and Events tabs no longer show a bare red dot. Backward-compatible — older audit files load unchanged. ([ORB-00026])
- **Gemini direct-agent sandbox + planner/arbiter artifact persistence**: narrow macOS sandbox allowances for Orbit child-runtime writes (global logs, DB sidecars, task workspace bundles, workspace semantic DB), tightened HOME-derived defaults, and improved planning-duel missing-artifact diagnostics so `orbit.duel.plan.add`/`winner` work under sandbox without home-directory re-allow. ([ORB-00027], [commit f3919a99], [commit e706d596])
- **Gemini CLI token accounting**: the shared response usage parser now reads `stats.models.<model>.tokens.{input,prompt,cached,candidates}` without double-counting role aggregates, so Gemini direct-agent invocations no longer persist as zero-token traces. ([ORB-00028])
- **Graph diagnostics quality**: `orbit.graph.show` now returns `did_you_mean` suggestions for unresolvable method selectors; exact-name `orbit.graph.search` ranks definition kinds (`trait`/`struct`/`enum`/`type`/`function`/`module`) above impl-method selectors; `orbit.graph.pack` carries typed `unresolved` reasons (`not_found` / `outside_indexed_roots` / `stale_snapshot`); `orbit.graph.overview` carries a typed `downgrade_reason` with threshold/actual. ([ORB-00029], [ORB-00030], [ORB-00031], [ORB-00032])
- **Task bundle creation no longer leaks a lock sentinel or double-dots its name**: `TaskBundleStoreV2::create_bundle` now locks on the bundle directory target and unlinks the `.ORB-XXXXX.lock` sentinel inside the locked closure; the old pre-dotted `create_lock_path` helper was removed. Concurrent-create serialization preserved. ([ORB-00033])
- **PR signature attribution corrected**: `batch_pr_signature` no longer falls back to `created_by` (which often names a planner or a human filer); when no task carries `implemented_by`, the signature falls back to the PR-opening agent's model identity, which is by construction the author of the commits. ([ORB-00034])
- **Dashboard AGENT LOGS rendering**: `<pre>` switched to `white-space: pre` with horizontal scroll so long single-line stdout (e.g. Codex JSON envelopes) and multi-word stderr stay legible instead of fragmenting one token per row. ([ORB-00035])
- **Dashboard friction-chip cleanup**: removed dead `friction` task-status references from `STATUS_ORDER` and the approve/reject eligibility sets. The diagnostics-tab friction surface (`/api/diagnostics/friction`) is untouched. ([ORB-00024])

### Chores

- **`make ci-fast` introduced for pre-handoff checks**: fmt-check + guardrail scripts, no compile. `make ci` stays the canonical merge gate via PR CI. Agent guidance updated to clarify when `make ci` failures classify as unrelated CI blockers vs task regressions. ([commit 4c22fa19], [commit 89ebc578])
- **Release docs cross-linked**: `RELEASING.md` and `docs/RELEASE.md` now reference each other so a first-time releaser landing on either file finds both. ([ORB-00040])
- **Deprecated activity/executor YAML assets removed**: cleanup of seeded resources that are no longer referenced by the runtime. ([commit c9cf36cc])
- **`devalue` bumped 5.7.1 → 5.8.1** in `website/`. ([commit f08446e1])

## 0.5.3

### Features

- **Claude Code plugin SessionStart hook**: the Orbit plugin now ships a `SessionStart` hook (`plugin/hooks/check-workspace.sh`) that detects an uninitialized workspace via a pure filesystem walk and surfaces a `systemMessage` instructing the user to run `orbit init` / `orbit workspace init`. Closes the silent-no-op gap where `orbit mcp serve` would attach with zero tools and no in-session signal. ([ORB-00018])

### Fixes

- **No-diff PR handoffs**: `pr_open` now treats branches with zero commits ahead of the configured base as successful repository-noop handoffs, moves completed tasks to `review`, returns `pr_created: false`, and avoids creating an empty GitHub PR. ([ORB-00016])

### Chores

- **Workspace lint guardrails**: codified Rust practice lints for panic surfaces, stdout/stderr usage, and async lock guards in `[workspace.lints]`, with scoped allowlists and `CLAUDE.md` updated to separate enforced rules from conventions. ([ORB-00013])
- **README workspace and MCP surface docs**: documented the `.orbit/` workspace layout, committed project-memory directories, and the agent-facing MCP tool namespaces. ([commit b99d3796])

## 0.5.1

### Fixes

- **Deprecation warning routes through `tracing`**: the `knowledge.task_id_pattern` deprecation notice was emitted via `writeln!` directly to stderr from `orbit-core`, contradicting the `CLAUDE.md` "`tracing` for diagnostics; `eprintln!` only in `orbit-cli`" rule and leaking into `cargo test` output. Now goes through `tracing::warn!` with a structured `config` field.

### Chores

- **Release metadata recovery**: backfilled the CHANGELOG entry for v0.5.0 (tagged without one) and aligned `Cargo.toml` workspace version + `Cargo.lock` to v0.5.1. v0.5.0 binaries shipped reporting `orbit --version` `0.4.0` because the workspace Cargo version was not bumped at tag time; v0.5.1 binaries report `0.5.1`. Tag-message and commit-message formats now follow `RELEASING.md`.

## 0.5.0

### Release scope

- **Task artifact v2 cutover**: the v1 single-file task store is replaced end-to-end by a bundle-based v2 store with status-neutral directories, append-heavy sidecar layout, a SQLite registry for fast lookups, a runtime backend, generated indexes, atomic delete, reservation scoping by workspace binding, search refinements (binary-artifact gating, UTF-8 validation), and a forward-only YAML migration framework keyed on `schema_version`. ADRs 001/002/003/004/007 for the v2 design flipped to Accepted; the v1 store, the legacy migration helpers, and the `[task] artifact_store = "legacy"` config gate are all gone.

### Breaking Changes

- **Legacy task artifact store removed**: the v1 single-file task store and its migration helpers were deleted in favor of the v2 bundle store. `[task] artifact_store` accepts `"v2"` as a no-op for forward compatibility and rejects `"legacy"` with an explicit migration error. Workspaces still on legacy storage must migrate under v0.4.0 before upgrading. ([commit e9582eba], [commit 222f6020], [commit 123f89f7])
- **`Task.workspace_path` / `Task.repo_root` dropped from the update path**: `TaskAutomationUpdate`, `TaskRecordUpdateParams`, and `TaskDocumentUpdateParams` no longer accept these fields, and the v2 document layer rejects them at load. Worktree setup and parallel-batch dispatch no longer thread them through. The fields remain on the public DTO via projection from workspace metadata, but write-path inputs that include them now fail. ([commit 6beb14a2])

### Features

- **ADR-artifact subsystem (`orbit.adr.*`)**: ADRs lift out of per-feature `4_decisions.md` markdown into first-class artifacts at `.orbit/adrs/<status>/<id>/{adr.yaml,body.md}` with globally-unique `ADR-NNNN` IDs and a three-state lifecycle (proposed/accepted/superseded). Ships domain types, a SQLite envelope index, a file store with per-ADR `fs2` locks, five tools (`add`, `show`, `list`, `update`, `supersede`) with lifecycle audit rows, an `orbit adr migrate` one-shot, and a parser/sweeper hardened against rollup-bullet form and code-fenced examples. Migration imported 142 ADRs (97 accepted / 39 proposed / 6 superseded) across the existing corpus. The envelope's `legacy_id: String` field was renamed to `legacy_ids: Vec<String>` mid-development per the ADR-002 amendment; pre-GA churn, no external consumers yet. ([T20260510-27], [T20260510-28], [T20260511-1], [T20260511-2], [T20260511-3], [T20260511-11])
- **Project learnings subsystem (`orbit.learning.*`)**: workspace-scoped `LearningFileStore` under `.orbit/learnings/<id>.yaml` with a SQLite `learnings_index` for fast scope-glob lookups on the injection hot path. Eight MCP tools (`add`, `list`, `search`, `show`, `update`, `supersede`, `reindex`, `prune`) plus matching `orbit learning <verb>` CLI; learnings travel with the repo via a carved-out `.gitignore` entry. End-to-end dispatch latency p95 ≈ 0.04 ms at 500 records. ([T20260511-5], [T20260511-6])
- **Plugin install contract locked down**: `make release-check` enforces version lockstep across `plugin/npm/package.json`, `plugin/.claude-plugin/plugin.json`, `npm view @orbit-tools/cli`, and `gh release list -L 1`; `scripts/smoke-plugin-install.sh` exercises the published `@orbit-tools/cli@latest` postinstall + MCP `tools/list` handshake; `.github/workflows/smoke-plugin-install.yml` runs the smoke weekly and on every `v*` tag on macOS and Linux; `docs/RELEASE.md` codifies the chain. v0.5.0 switched the npm publish step to manual after the workflow ran into account-level 2FA. ([ORB-00012], [ORB-00014])
- **Typed error discriminators**: `KnowledgeError.kind` is now the typed `KnowledgeErrorKind` enum, and ten per-kind `OrbitError::*NotFound(String)` variants consolidate into `NotFound { kind: NotFoundKind, id: String }` with exhaustive matches in every error-code translator. JSON wire shape is preserved via `#[serde]`; internal Rust matchers move to the typed kind. ([ORB-00001])
- **Property + snapshot tests on protocol boundaries**: proptest-backed `Selector` display/parse roundtrip (256 cases per variant), multi-threaded `GraphLockGuard`/`LockStore` concurrency with a 5-second deadline, and committed JSON-snapshot coverage for `AuditGuard` success/failure/denied events. ([ORB-00002])
- **Knowledge graph workflow relocation**: `build`/`show`/`search`/`history` workflows moved from `orbit-tools` into a new `orbit_knowledge::workflows` module, so `orbit-knowledge` owns both the tool surface (`commands/`) and the host application surface (`workflows/`). ([commit 5fc2b72c])
- **Forward-only YAML migration framework**: `orbit_common::migration::Plan` chains `Value → Value` steps keyed on `schema_version`; `OrbitError::Migration` separates chain failures from store/parse errors; the v2 task-bundle envelope read path now flows through `task_migrations::envelope_plan()` (empty chain today — next schema bump adds one `add_step` call). ([commit 01928e76])
- **Semantic companion install hardening**: companion installation is version-aware, `--force` reinstalls supported, and background-companion stderr is suppressed during task-mutation indexing. ([T20260510-26])

### Fixes

- **`pr_open` template tolerates legacy `batch_id` overrides**: `worktree_setup` emits `batch_id` as a deprecated alias equal to `job_run_id`, and `required_job_run_id` accepts `job_run_id` → `run_id` → legacy `batch_id` in that precedence. Recovers `task_pr_pipeline` runs whose stale resource overrides still reference `{{ steps.worktree.output.batch_id }}`. ([ORB-00010])
- **Retired v1 / stub paths fail loudly instead of silent success**: `OrbitRuntime::get_job` and `RuntimeHost` v1 job lookup return an explicit retired-v1 error instead of `Ok(None)`; the `promote_agent_main` and `revert_on_red` deterministic actions fail with a "retired stub" error instead of returning skipped JSON. Tightens validation on inputs that were already invalid by spec. ([ORB-00007])
- **Architecture and design-doc drift fixed**: `ARCHITECTURE.md` was regenerated from `cargo metadata` to reflect the live workspace graph; `make check-design-docs` was cleared by refreshing stale `Last updated:` metadata and replacing moved/deleted file references across ten design folders. ([ORB-00006])
- **Smoke plugin-install assertion fixed**: assertion was checking `"orbit\.` but MCP wire names use underscores (`sanitize_tool_name` in `orbit-mcp/src/adapter.rs` replaces `.` with `_`). Without this fix the on-tag smoke would have false-failed. ([ORB-00014])

### Chores

- **Per-crate stability tier markers**: `[package.metadata.orbit] stability = ...` added to all fourteen workspace crates (`stable`: `orbit-common`, `orbit-store`; `experimental`: `orbit-embed-companion`, `orbit-registry`; `internal`: the remaining ten). `scripts/check-stability.sh` enumerates members via `cargo metadata`, validates the marker, and fails closed with named offenders; wired into `make stability` and `make ci`. ([ORB-00005])
- **Missing-docs guardrail**: workspace-wide `missing_docs = "warn"` in `[workspace.lints.rust]`; `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --workspace` wired into `scripts/ci-guardrails.sh`; 278 accidentally-`pub` items narrowed to `pub(crate)` (6.8% of the 4,116-item baseline). Legacy allow-fences in place for the remainder. ([ORB-00004])
- **Community health files**: `CODE_OF_CONDUCT.md` (Contributor Covenant v2.1, contact via GitHub Security Advisories), `.github/PULL_REQUEST_TEMPLATE.md` with linked Orbit task ID + `make ci` / `make check-design-docs` checkboxes, and three `.github/ISSUE_TEMPLATE/*.yml` issue forms with a 14-crate dropdown sourced from live `crates/` listing. ([ORB-00011])
- **Release runbook (`RELEASING.md`)**: pre-1.0 versioning policy with explicit breaking-vs-non-breaking criteria, 11-step release checklist, CHANGELOG conventions, and tag-push CI workflow description. ([T20260510-24])
- **Semantic search surfaced in agent instructions**: new `orbit-semantic` SKILL.md modeled on `orbit-graph`, plus pointers from `orbit-create-task`, `orbit-execute-task`, `orbit-review-task`, `agent_implement.yaml`, `agent_review.yaml`, `epic_orchestrator.yaml`, and `dispatch_agent.yaml`. All references use "if available" / "optional" language so missing companion never hard-fails a workflow. ([T20260511-4])
- **Design-pattern reference docs added**: `docs/design-patterns/{command,strategy,raii_guard,newtype,error_translation}.md` so feature work can copy from documented references instead of inventing new shapes. ([commit 3adcd838], [commit c713cc30], [commit 66389575], [commit f2f82bf1], [commit aa407aa0])
- **Design-doc decay check**: new `scripts/check_design_doc_decay.py` and `make check-design-docs` flag `docs/design/*` docs whose `Last updated:` precedes the last commit on any referenced `crates/.../*.rs` file. ([commit 18c48744])
- **Workspace lint table introduced**: `[workspace.lints]` in root `Cargo.toml` with each crate inheriting via `lints.workspace = true`; mechanical lint rules moved out of `CLAUDE.md` prose. ([commit 0cbb037d])
- **`CLAUDE.md` refactored to point at `ARCHITECTURE.md`**: crate layering moved to a dedicated architecture doc; `CLAUDE.md` shrinks to project rules and judgment calls. ([commit b7c590aa], [commit 9362730b], [commit 7582af9d])
- **MCP server configuration for additional environments**: `.codex/config.toml`, `.gemini/settings.json`, and `.vscode/mcp.json` added so Codex/Gemini/VS Code agents pick up Orbit MCP out of the box. ([commit 200ee6fd])
- **README simplification**: Quick Start trimmed; positioning sentences moved to design docs. ([commit 75699824])
- **Agent skills symlink**: `.agents/skills` symlink added so external agent harnesses pick up the seeded skill set. ([commit 9f7bf89b])
- **Lessons log update**: `docs/LESSONS.md` extended with a workflow lesson. ([commit 024013af])

## 0.4.0

### Release scope

- **Pivot to "auditable agentic task management"**: README and landing-page positioning realigned around intent attribution and audit trails, with throughput and parallel-execution sections refreshed to match.
- **Knowledge graph reads on SQLite**: per-build `graph_index.sqlite` sidecar with read-only fast paths for `graph.overview`, `graph.search`, and `graph.show`, plus an output-equivalence harness against the JSON fallback.
- **Semantic search foundation (preview)**: hybrid (BM25 + cosine + RRF) retrieval over tasks, delivered as a separately-installed `orbit-embed-companion` binary. Preview status — surface may change before v1.

### Breaking Changes

- **Friction reports relocated**: friction is no longer a task type or status. Records live as append-only markdown under `.orbit/frictions/{yyyy}-{mm}/F{nnn}.md` and are managed through `orbit.friction.add/list/show/stats`. `orbit.task.add` rejects `type: friction` / `status: friction`; web API and scoreboard JSON drop `friction_bounty`. ([T20260510-13])
- **Task type taxonomy reduced**: the `task | feature | epic | issue | bug | chore | refactor | friction` enum collapses to `feature | bug | refactor | chore`. `orbit.task.add` and `orbit.task.update` reject the removed values; existing tasks were migrated. ([T20260510-14])
- **Attribution narrowed to `model`**: the `agent` field is removed from `Actor`; `orbit.task.add` rejects an `agent` parameter and Orbit infers the agent family from `model` via `agent_from_model`. MCP `orbit_task_list`, `orbit_task_search`, and `orbit_task_review_thread_list` responses are now object-shaped (previously top-level arrays) so Cursor and VS Code accept them. ([T20260510-15])
- **Knowledge-graph leaf IDs unified across extractors**: Python, Rust, Java, and TypeScript leaf selectors now use a single canonical form so SQL and JSON paths return set-equivalent results. `GRAPH_SQLITE_INDEX_SCHEMA_VERSION` bumps; consumers caching selectors must rebuild. ([T20260510-7])
- **Semantic search requires a companion binary**: `orbit-embed-companion` is installed separately via `orbit semantic install`; `orbit semantic *` and the matching MCP tools fail until it is present. ([T20260510-9], [T20260510-10])
- **`JobV2Step` rejects multi-body shapes**: YAML steps that previously parsed silently with both `target` and `parallel` (or any other body combination) now fail at load. ([T20260509-31])
- **`orbit-locks` skill removed**: the seeded `orbit-locks/SKILL.md` and the ad-hoc `orbit.task.locks*` instructions in the seeded `orbit` skill are gone — the gate pipeline still owns reservations. External agent prompts referencing the skill must be updated. ([T20260510-17])

### Features

- **Knowledge graph SQLite read facade**: per-build `graph_index.sqlite` with versioned schema, read-only facade with graceful JSON fallback, and SQL fast paths for `graph.overview` (aggregation), `graph.search` (exact-name, path-prefix, and substring), and `graph.show` (selector lookup with `children` repopulated via a forward-pointer edge table). ([T20260509-70], [T20260509-71], [T20260509-72], [T20260509-73], [T20260509-74])
- **Knowledge graph latency wins**: lazy source hydration via `GraphReadOptions`, a bounded default-ranking work cap on search, and a `BinaryHeap` top-K in `overview.top_files`. ([T20260509-65], [T20260509-67], [T20260509-68])
- **Knowledge command surface**: graph business logic — ranking, classification, fast-path orchestration — relocated into `orbit_knowledge::commands::*` so non-tool consumers share canonical behavior. ([T20260510-5])
- **Semantic search subsystem**: `orbit-embed` client, `orbit-embed-companion` binary, `embeddings` and `tasks_fts` SQLite schema, paragraph chunker, BLAKE3 dedup, task-mutation index hooks, and `orbit semantic install/uninstall/reindex/stats/search/related` CLI plus MCP surface. ([T20260510-3], [T20260510-9], [T20260510-10], [T20260510-20])
- **Task tags**: first-class `tags: Vec<String>` field with normalized SQLite index and `--tag` filtering on `orbit task list/search`. ([T20260510-12])
- **Activity/job runtime polish**: wildcard-aware tool allowlists honored at dispatch and HTTP-loop schema advertisement, asset-load-time allowlist validation, agent-loop `on_denial: continue`, literal-boolean condition atoms, and exclusive locking on duel scoreboard appends. ([T20260509-15], [T20260509-22], [T20260509-23], [T20260509-25], [T20260509-32])
- **Recovery role configurability**: seeded step-failure recovery activity uses `role: reviewer` and resolves agent/model from `[agent.reviewer]` config instead of hardcoded Codex. ([T20260509-14])
- **Done-task sync cap**: website task sync caps generated pages to the 100 most recent `done` tasks. ([T20260509-20])
- **Debug-job-failure skill**: seeded `orbit-debug-job-failure` SKILL.md teaching agents how to investigate failed/stuck/cancelled job runs across run state, audit events, blobs, and live processes. ([T20260509-79])
- **Graph-latency benchmark**: split `benchmarks/CONVENTIONS.md` into agent vs perf RESULTS schemas, scaffolded `benchmarks/graph-latency/` with three-tier Python/Java/Rust corpora, and ran v1/v2 sweeps against the post-SQLite read paths. ([T20260509-63], [T20260509-87], [T20260510-4])

### Fixes

- **Output-equivalence between SQL and fallback paths**: `graph.search` SQL widened to substring match aligned with the navigator and `graph.show` repopulates `children` via a forward-pointer edge table. ([T20260510-1], [T20260510-2])
- **Workflow stop on implementer envelope failure**: `peek_response_status` extracts embedded Orbit envelopes from CLI stdout that contains explanatory prose before the JSON, so failed implementations no longer advance to push/PR. ([T20260509-15])
- **`ship-auto` empty backlog**: condition evaluator skip guards no longer fail when `bundle_count` is zero. ([T20260509-11])
- **Parallel dispatcher hang on worker timeout**: scoped-thread workers now exit through a cancellable boundary so the pipeline returns within its own timeout. ([T20260509-38])
- **Subprocess timeout cleanup**: bare `spawn_with_timeout` starts children in a process group/session so grandchildren are killed and pipes don't leak. ([T20260509-40])
- **Stdout no longer duplicated into `DispatchOutcome`**: blob refs are the source of truth, with a bounded preview retained. ([T20260509-43])
- **Path-traversal hardening**: task store ID validation, policy candidate-path component checks, resource-name validation in policy/executor stores, and an absolute-path probe for `sandbox-exec`. ([T20260509-26], [T20260509-27], [T20260509-28], [T20260509-30])
- **Tool deletion guard**: `orbit.task.delete` MCP tool respects the same protected-status guard as `orbit task delete`. ([T20260509-44])
- **Backend resolution**: invalid `[runtime] backend` values reject before dispatch instead of falling through to preview HTTP. ([T20260509-45])
- **Architecture guardrail**: `scripts/check-dependency-direction.sh` derives the workspace-crate list from `cargo metadata` so new crates can't drift past the check. ([T20260509-46])
- **JSON output purity**: `orbit task approve --all-proposed --json` and reject equivalents emit pure JSON on stdout. ([T20260509-47])
- **Dashboard dependencies**: dependency-status index includes `done` and `archived` tasks so visible rows don't misreport completed deps as missing. ([T20260509-48])
- **Reject help/help-truth alignment**: top-level task help describes the actual reject transition matrix. ([T20260509-50])
- **Symlink scanning**: knowledge scanner skips and canonicalizes symlinked dirs to prevent index escape and cycles. ([T20260509-33])
- **Graph freshness**: manifest persists exact Git identity rather than relying on committer timestamp. ([T20260509-34])
- **GitHub PR result validation**: `github.pr.review` and `github.pr.comment.reply` validate JSON shape before reporting success with id `0`. ([T20260509-36])
- **MCP name collisions**: dot-to-underscore name mapping detects ambiguity on startup. ([T20260509-37])
- **Git author identity**: workflow commits set per-implementer author dynamically without writing repo-local `git config user.*`. ([T20260508-22], [T20260509-12])
- **CI clippy guardrails**: cleared `manual_contains`, `needless_borrow`, `useless_conversion`, `match_like_matches_macro`, `empty_line_after_doc_comments`, `too_many_arguments`, `doc_lazy_continuation`, and `question_mark` violations under `-D warnings`. ([T20260509-18], [T20260509-61], [T20260510-15], [T20260510-22])
- **`fast-uri` Dependabot alerts**: addressed and documented dev-only `fast-uri` advisories on `website/package-lock.json`. ([T20260509-57])

### Chores

- **Module decomposition**: split `command/web/api.rs` (3,376 LOC), `activity_job/job_executor.rs` (2,841 LOC), `activity_job/cli_runner.rs` (2,161 LOC), `runtime/orbit_tool_host/mod.rs` (2,033 LOC), `command/mcp/setup.rs` (1,964 LOC), and the `activity_job/groundhog.rs` runner. ([T20260509-1], [T20260509-2], [T20260509-3], [T20260509-4], [T20260509-5], [T20260509-19])
- **Embed crate ownership**: relocated `vector::*` and the semantic command surface from `orbit-store` and `orbit-core` into `orbit-embed`, reversing the dep arrow so `orbit-store` no longer knows the embedding feature exists. ([T20260510-20])
- **Panic audit**: classified ~1,864 `unwrap` / `expect` / `panic!` sites and removed the accidental ones in execution-critical paths. ([T20260509-6])
- **Test coverage on highest-risk seams**: focused tests for the activity/job DAG executor and the macOS sandbox/policy boundary. ([T20260509-7])
- **Plan-duel `context_files` extraction**: duel resolver auto-populates `task.context_files` from the winning plan's Context Files section. ([T20260509-9])
- **Knowledge-graph, policy-sandbox, and Groundhog doc hygiene**: refreshed owned design docs to current surface.
- **Task lineage design (first draft)**: seeded `docs/design/task-lineage/` with edge schema, three derivers, bipartite bridge, `feature` closure, and symbol-biography renderer. ([T20260510-21])
- **Project learnings design (seed)**: seeded `docs/design/project-learnings/` with hook-injection layer rationale; deferred until semantic search is Accepted. ([T20260510-11])
- **Semantic search v2 design pivot**: switched to companion-binary architecture per ADR-005. ([T20260510-3])
- **Orbit-create-task skill**: tightened `context_files` rule (existing modified or deleted files only, prefer file-level selectors). ([T20260509-83])
- **`make ci` alignment**: `make build` failures resolved alongside the dep-direction script. ([T20260510-22])
- **Release metadata**: bumped Cargo workspace, plugin manifests, and npm proxy metadata to v0.4.0.

## 0.3.1

### Features

- **Pipeline dispatch reliability**: hardened parallel, gate, and epic pipelines with failed-child completion handling, longer task lock coverage, epic timeout/convergence fixes, resolved workspace subprocess cwd, and per-step agent log/error surfacing. ([T20260427-34], [T20260427-36], [T20260427-38], [T20260427-40], [T20260508-8], [T20260508-14])
- **Metrics and public docs**: split the public metrics surface into Operations and Scoreboard views, added done-task sync pages for orbit-cli.com, refreshed positioning/reference docs, and refined the website UI. ([T20260508-4], [T20260508-16], [T20260508-19], [T20260508-20], [T20260507-21])
- **Registry and benchmark tooling**: added the `orbit-registry` crate and identity-key benchmark harness for exercising knowledge graph selector stability. ([T20260507-12], [T20260508-2])

### Fixes

- **macOS sandbox and CLI execution**: allowed Claude's `$HOME/.claude.json` lock/tmp siblings, re-allowed the active job-run worktree after global deny rules, and demoted successful CLI exits when the inner Orbit envelope reported failure. ([T20260508-13], [T20260508-17])
- **Workflow defaults and links**: made workflow base branches resolve from `[workflow] base_branch` when CLI flags are omitted, and fixed task-ID links in generated PR bodies with an opt-in URL template. ([T20260508-11], [T20260508-12])
- **CI clippy guardrails**: grouped macOS sandbox spawn inputs into a request struct so strict workspace clippy passes under `-D warnings`. ([T20260508-21])

### Chores

- **Release metadata**: bumped Cargo workspace crates, plugin manifests, install examples, and npm proxy metadata to v0.3.1. ([T20260508-21])
- **Release packaging**: kept GitHub Release tarballs, checksums, Homebrew tap updates, and installer smoke tests as the supported release path, while removing the npm publish step from the tag workflow.

## 0.3.0

### Release scope

- **Stable surface: CLI agent backends.** v1 supports `backend: cli` as the stable agent invocation path, running Codex, Claude Code, Gemini CLI, and other official CLIs as supervised subprocesses. `backend: http` (`LoopTransport`) and the Groundhog checkpoint runner remain preview-only for v1; they are exercised in tests but can change before v2.

### Breaking Changes

- **Activity/job schema v1 removed**: loaders now reject `schemaVersion: 1` activity/job assets, the v1 reconcile/runtime/store paths are gone, and `schemaVersion: 2` is the canonical activity/job surface. ([T20260419-2156], [T20260420-0036])
- **Workflow commands reorganized**: stable entrypoints are `orbit run ship <TASK_ID>...`, `orbit run ship --mode local <TASK_ID>...`, `orbit run ship-auto`, `orbit run duel-plan <TASK_ID>`, and `orbit run job <JOB_ID>`. The direct `orbit run <JOB_ID>` shorthand and workflow-specific `run ship list/show` and `run duel list/show` commands were removed; use `orbit run history` and `orbit run show` for job-run inspection. ([T20260417-0248], [T20260419-0355], [T20260425-2010], [T20260426-0742])
- **Task attribution history moved to `orbit graph history`**: selector history is graph-owned, so the query now lives next to `orbit graph search/show`, and rebuilds use `orbit graph build`. Both `orbit graph build` and `orbit graph history` accept `--task-id-pattern <regex>`; workspace config `knowledge.task_id_pattern` is the steady-state setting, with CLI flag > config > Orbit default precedence. The selected pattern is recorded in `manifest.json`, and mismatches emit a stderr warning. `orbit.graph.history` exposes the same surface to MCP clients. ([T20260426-0507])

### Features

- **Activity/job v2 runtime**: added schema v2 activities and jobs with typed DAG blocks (`parallel`, `fan_out`, `loop`, `retry`, `when`), activity name resolution, `backend: auto` normalization, `backend: cli` dispatch, HTTP agent loops, session-bound loop steps, and a v2 audit envelope with workspace provenance. ([T20260418-2018], [T20260418-2019], [T20260418-2143], [T20260418-2210], [T20260419-0002], [T20260419-0104])
- **Seeded task pipelines**: added load-bearing seeded workflows for PR, local, gate, auto-dispatch, and epic shipment, including task reservations, backlog bundling, admission-controlled dispatch, and session-backed epic orchestration. ([T20260419-0622-3], [T20260419-0623], [T20260419-0623-2], [T20260419-2347])
- **Knowledge graph**: added the Rust `orbit-knowledge` graph, `orbit graph build/update/search/show`, graph MCP tools, compact overviews, callers/implementors/dependency navigation, edit buffering, shared locks, auto-refresh, branch-scoped refs, task-ID attribution metadata, and markdown/config/table extraction. ([T20260411-0424], [T20260412-0645-2], [T20260412-0645-3], [T20260421-0358], [T20260421-0528], [T20260422-1540])
- **MCP integrations**: added the `orbit-mcp` crate, `orbit mcp serve`, safe default graph/task tool exposure, external MCP/plugin tooling, and `orbit mcp init/remove` setup for Claude, Codex, and Gemini clients. ([T20260418-0336], [T20260419-0236], [T20260422-1713], [T20260426-0354])
- **Dashboard and observability**: added `orbit web serve`; task, job, audit, scoreboard, and dashboard APIs; diagnostics and recent-runs views; task actions; copyable task IDs; connection health; skeleton/loading states; markdown rendering; and live-data animations. ([T20260417-0346], [T20260417-0412], [T20260417-0427], [T20260417-0437], [T20260417-0528], [T20260418-2004], [T20260426-0354])
- **Task planning and search**: added structured task plans, dependency support, epic task type support, selector-first task context, agent task search, and richer task field projection for agent/tool callers. ([T20260419-2300], [T20260420-0509-2], [T20260420-0521], [T20260421-0445], [T20260422-1756])
- **Groundhog execution model (preview)**: added Groundhog chronicle serialization, workspace snapshots, verb tools, checkpoint verification, and a dedicated Groundhog v1 activity runner. ([T20260420-0509], [T20260420-0509-3], [T20260420-0509-4], [T20260420-0510], [T20260420-0510-2])
- **Provider and evaluation support**: added Gemini support, configurable agent/model selection, provider invocation traces, HTTP LoopTransport implementations for Anthropic/OpenAI-compatible/Gemini providers, planning duels, scoreboard attribution improvements, and versioned knowledge-graph benchmark harnesses. ([T20260411-1937-2], [T20260412-0457-2], [T20260412-1939], [T20260412-2129], [T20260418-0645], [T20260418-0759], [T20260422-1609])

### Fixes

- **Job run observability**: `orbit run ship --json`, `orbit run history`, `orbit run show`, and direct `orbit job run` now retain actionable failure details and durable run-state/job-history records, including synthetic job-level steps for early v2 pipeline failures. ([T20260423-0445], [T20260423-2004-4], [T20260425-2010], [T20260426-0742])
- **Branch-scoped knowledge graph refs**: graph builds now write `.orbit/knowledge/graph/refs/heads/<branch>.json` files that point at immutable per-build indexes, reads default to the current git branch with default-branch fallback, and legacy `.orbit/knowledge/graph/refs/current.json` stores auto-migrate on first open/write. ([T20260421-0358])
- **Knowledge graph hardening**: graph reads and refreshes recover from corrupted stores, avoid stale worktree data, gate refresh/search hot paths, prune missing context files from locks, and hydrate task IDs idempotently during attribution. ([T20260416-0719], [T20260417-0307], [T20260420-0540], [T20260421-0652])
- **Dispatch and locking correctness**: task locks now detect directory/file overlaps, backlog selection filters locked groups, failed task-scoped runs move tasks to blocked with job/run/error context, and drained local batches no longer fail spuriously. ([T20260412-0443], [T20260417-0301], [T20260419-2109], [T20260420-0014])
- **Workflow compatibility**: merged object-valued job defaults with caller input, aligned the Quick Start approval flow with the current task lifecycle, and routed retired workflow inspection docs/errors to `orbit run history/show`. ([T20260423-0445], [T20260423-0447], [T20260423-2004-2], [T20260425-2010], [T20260426-0742])
- **Release and developer tooling**: restored release CI targets, repaired advertised developer targets, kept custom roots isolated, and fixed crashes/empty listings after seeded activity/job initialization. ([T20260419-2347], [T20260423-2004], [T20260423-2004-3], [T20260423-2004-5])
- **Security and concurrency hardening**: added localhost origin checks for web write endpoints, serialized diagnostics JSONL appends, hardened task-store concurrency, tightened filesystem/tool-runtime path boundaries, and strengthened agent protocol handling. ([T20260417-0557], [T20260417-0558], [T20260418-1928])

### Chores

- **Crate architecture**: extracted `orbit-common`, `orbit-knowledge`, and `orbit-mcp`, merged the older `orbit-types` surface into `orbit-common`, decomposed execution/runtime modules, and kept crate dependency direction aligned with the documented architecture. ([T20260411-0008], [T20260419-2014])
- **Documentation and positioning**: added Orbit positioning docs, design-doc conventions, activity-job/knowledge-graph/Groundhog design docs, benchmark reports, and README updates for the current workflow and MCP surfaces.

## 0.2.0

### Features

- **Parallel batch execution**: dispatch and execute multiple tasks in parallel with file-level conflict detection and shared worktrees
- **Auto-cleanup on merge**: ship workflow now deletes the remote branch after a successful PR merge

### Fixes

- **`--parallelism` flag**: serialized as JSON integer instead of string, fixing schema validation failure on `orbit run ship --parallelism N`
- **Stale default artifacts**: `orbit workspace init` now always refreshes default skills, activities, and jobs to their latest embedded versions (custom artifacts are preserved)
- **Clippy warning**: resolved unused-mut warning and removed clippy from CI

### Chores

- Default branch renamed from `agent-main` to `main`
- Removed `orbit` label from PR creation
- Agent configuration updates

## 0.1.0

Initial release of Orbit.

### Core

- **Task lifecycle**: propose, approve, implement, review, and archive tasks with full history tracking
- **Activity system**: reusable operations with defined input/output schemas and three spec types (agent_invoke, cli_command, automation)
- **Job engine**: composable multi-step pipelines with conditional execution, retry logic, nested jobs, and parallel dispatch
- **Workflow aliases**: `orbit run ship`, `orbit run ship-local`, `orbit run review` as ergonomic entry points over raw job invocation
- **Multi-agent orchestration**: parallel task workers with file-level locking in shared worktrees
- **Multi-model strategy**: configurable agent/model per job step (e.g., Opus for planning, Codex for implementation)

### CLI

- Grouped command surface: run workflows, manage work, configure and inspect
- JSON and table output modes across all commands
- Audit event logging for every CLI invocation

### Infrastructure

- Layered Rust crate architecture (types, policy, exec, tools, store, agent, engine, core, cli)
- Two-root workspace model: global (`~/.orbit/`) and workspace-local (`.orbit/`)
- File-based (YAML) and SQLite persistence
- RBAC policy evaluation engine
- Process sandboxing and timeout handling
- Skill system for agent prompt composition
