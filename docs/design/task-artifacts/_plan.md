# Task Artifacts Execution Plan

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-11

This is the temporary execution tracker for the task-artifacts reset until Orbit tasks can track this work again. It intentionally assumes a pre-release reset: existing task artifacts were deleted, no external users depend on the old layout, and the implementation does not need a migration command, legacy lookup aliases, or old-schema compatibility shims.

**Temporary exception:** this file is not stable design-contract material and does not belong in `references/`. It should be deleted or archived once Orbit task artifacts can track the remaining implementation work again.

## Current Position

The design baseline is in place:

- `docs/design/task-artifacts/1_overview.md` defines the feature intent.
- `docs/design/task-artifacts/2_design.md` defines the target v2 layout.
- `docs/design/task-artifacts/specs/task-bundle-v2.md` defines the storage contract.
- `docs/design/task-artifacts/4_decisions.md` records the proposed ADRs.

Implementation has started. Phase 0 and Phase 1 landed in `c1f72a32`. Phase 2 landed in `1ae83804`. Phase 3 landed its first bundle-primitive slice in `c14fa640`; the current working-tree slice adds the v2 task backend adapter, document/history/review/artifact mutations, trait wiring, config-gated runtime construction via `[task] artifact_store = "v2"`, and the first generated registry-index slice. Delete, public relation query wiring, full-text indexes, and task-lock rekeying remain open.

## Non-Goals

- No `orbit migrate task-artifacts` command.
- No support for old `T<YYYYMMDD>-<N>` task IDs in v2 lookup paths.
- No old-to-new ID mapping report.
- No status-directory reader fallback.
- No embedded `description`, `acceptance_criteria`, `history`, `comments`, or `review_threads` compatibility inside v2 `task.yaml`.
- No dual-write mirror between `.orbit/tasks/` and `~/.orbit/tasks/`.

## Phase 0 - Prep And Guardrails

**Status:** Implemented in working tree

Goal: make the reset explicit in tests, docs, and compile-time surfaces before changing storage.

Work:

- Add v2 fixture examples for a minimal task, task with relations, task with comments, and task with review threads.
- Add tests that reject old task IDs and old task envelope fields in v2 code paths.
- Identify all code paths that currently assume status directories, date partitioning, or `T...` IDs.
- Decide whether `Task` remains the temporary public DTO during the cutover or whether a new v2-facing DTO lands first.
- Update design docs if implementation discovers a contract mismatch.

Exit criteria:

- Fixture set is present under `crates/orbit-common/tests/fixtures/task-artifacts-v2/`; commit pending.
- Search inventory exists for old-layout assumptions. Complete as of 2026-05-11.
- The implementation branch has passing guardrail tests for the intended v2 behavior.

### Phase 0 Inventory

Current old-layout assumptions are concentrated in these surfaces:

- `crates/orbit-store/src/file/task_store/layout.rs` owns status directories, terminal-month partitioning, `T<YYYYMMDD>-<N>` allocation, ID validation, path lookup, and legacy status-dir migration.
- `crates/orbit-store/src/file/task_store/api.rs` creates tasks through the old allocator, lists by scanning status directories, validates old IDs at public store entry points, and mutates the flat bundle in memory before rewriting.
- `crates/orbit-store/src/file/task_store/doc.rs` embeds `description`, `acceptance_criteria`, `history`, `comments`, `review_threads`, `workspace_path`, and `repo_root` inside `task.yaml`.
- `crates/orbit-store/src/file/task_store/bundle.rs` reconstructs the public `Task` DTO from `task.yaml`, `plan.md`, and `execution-summary.md`.
- `crates/orbit-store/src/file/task_store/lock.rs` still keys file locks by the old task ID validator.
- `crates/orbit-common/src/types/task.rs` remains the flat public DTO and still carries old embedded fields, legacy relation fields, workspace bindings, comments, history, and review threads.
- `crates/orbit-core/src/command/task/*`, `crates/orbit-tools/src/builtin/orbit/task/*`, and `crates/orbit-tools/src/builtin/orbit/review_thread/*` expose the flat task DTO and old field names at CLI/tool boundaries.
- `crates/orbit-store/src/sqlite/task_reservation_store.rs` and `crates/orbit-core/src/runtime/orbit_tool_host/task_locks.rs` are the current lock-reservation surfaces that Phase 4 must re-key to workspace binding plus canonical task IDs.

DTO decision for cutover: keep the existing `Task` as the runtime/API compatibility DTO until the v2 bundle store can project from the new artifact model. Add `TaskEnvelopeV2` and sidecar/log/thread/manifest types as storage-domain contracts first. This avoids forcing every CLI, MCP, engine, and web consumer to move before persistence exists, while still preventing new v2 code from accepting the old YAML envelope.

## Phase 1 - V2 Domain Types

**Status:** Implemented in working tree

Goal: define the storage model without touching persistence yet.

Work:

- Add `ORB-00000` validation and formatting helpers.
- Add `TaskEnvelopeV2` with `schema_version: 1`.
- Add `TaskRelation` and relation validation for type set, self-edges, duplicates, and blocking/hierarchy cycles.
- Add JSONL row structs for task events and comments.
- Add review-thread metadata structs with `schema_version: 1`.
- Add artifact manifest structs with checksum, media type, size, and blob path.
- Add document-name constants for `description`, `acceptance`, `plan`, and `execution-summary`.

Exit criteria:

- V2 structs serialize and deserialize to the documented shape.
- Unit tests cover ID validation, relation validation, schema-version presence, JSONL rows, and artifact path validation.
- No production command writes v2 bundles yet.

## Phase 2 - Home Registry And Workspace Binding

**Status:** Implemented in working tree

Goal: make `~/.orbit/tasks/index.sqlite` and `.orbit/config.yaml` load-bearing.

Work:

- Add home task registry creation under `~/.orbit/tasks/index.sqlite`.
- Add allocator state with a machine-local monotonic `ORB-*` suffix.
- Add workspace bindings table with `workspace_id`, slug, repo root, workspace path, `.orbit` path, and optional fingerprints.
- Add `.orbit/config.yaml` read/write with `schema_version: 1` and `workspace_id`.
- Add workspace-id assignment as `<slug>-<6char>`.
- Add rebind detection for missing `.orbit/config.yaml`; ambiguous matches must prompt or error, not silently attach.
- Add projection rebuild support for `.orbit/tasks/<task-id>` symlinks.
- Define and implement the first degraded-mode behavior for symlink-restricted filesystems.

Exit criteria:

- A fresh checkout can initialize a workspace binding.
- Multiple local workspaces allocate from one local ID authority.
- Deleting `.orbit/tasks/` can be repaired from registry metadata.

## Phase 3 - V2 Bundle Store

**Status:** Implemented in working tree

Goal: replace status-directory storage with canonical home bundles plus symlink projection.

Work:

- Introduce the v2 bundle abstraction around:
  - `task.yaml`
  - `description.md`
  - `acceptance.md`
  - `plan.md`
  - `execution-summary.md`
  - `events.jsonl`
  - `comments.jsonl`
  - `review-threads/`
  - `artifacts/`
- Write new tasks directly to `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/`.
- Create `.orbit/tasks/<task-id>` symlinks after canonical bundle creation succeeds.
- Implement atomic sidecar rewrite helpers.
- Implement JSONL append helpers with flush, file sync, and final-line tail-corruption recovery.
- Make list/get locate tasks through the registry and canonical paths rather than status-directory scans.
- Remove lifecycle directory moves; status changes update the envelope and append an event.

Exit criteria:

- `task add`, `task show`, `task list`, and basic status updates work on v2 bundles.
- No task operation requires status directories.
- A task bundle can be inspected manually and matches the spec.

### Phase 3 Progress

Implemented in working tree:

- Private v2 bundle abstraction under `crates/orbit-store/src/file/task_store/v2_bundle.rs`.
- `TaskV2Store` adapter under `crates/orbit-store/src/file/task_store/v2_store.rs` for create/get/list/filter/search plus document, history, review-thread, and artifact mutations.
- `workspace_task_backends_v2` constructor that returns the normal task backend trait bundle over the v2 adapter.
- Config-gated runtime construction through `[task] artifact_store = "v2"`.
- Durable creation for `task.yaml`, Markdown sidecars, `events.jsonl`, `comments.jsonl`, `review-threads/`, and `artifacts/files/`.
- Registry-backed canonical bundle paths, task-bundle registration, and workspace projection rebuild after bundle creation.
- Missing `.orbit/config.yaml` rebind detection uses registry candidates before creating a new workspace binding.
- Per-task create locking so same-ID bundle creation serializes before the existence check.
- Projection rebuild failures after registration return degraded success rather than exposing a failed create for committed state.
- Durable document rewrites through `atomic_write_text`, including `description.md`, `acceptance.md`, `plan.md`, `execution-summary.md`, review-thread YAML/Markdown, and artifact manifests.
- JSONL append helpers with sibling file lock, tail repair, flush, and file sync.
- Typed missing-bundle handling maps missing `task.yaml` to `TaskNotFound` instead of error-string matching.
- Runtime smoke coverage for add/show/list/search/start/update in v2 mode and workspace binding reuse across runtime rebuilds.
- Tests for round-trip storage, review-thread files, registry-backed listing, document rewrites, history/comment/status appends, review-thread merge/replace, artifact manifest writes, append behavior, tail repair, corruption rejection before the tail, concurrent appends, rename invariant rejection, projection degraded success, and cleanup after failed creation.

Known Phase 3/4 caveat: v2 single-file writes are atomic, but multi-file mutations are not transactional across files. The store keeps partial states readable; repair/indexing passes must reconcile cases such as events appended before envelope status rewrite or artifact files written before manifest rewrite.

## Phase 4 - Task Operations And Local Indexes

**Status:** In progress

Goal: restore the full task workflow on top of v2 storage.

Work:

- Update document updates to write Markdown sidecars. Done in working tree.
- Update comments to append `comments.jsonl`. Done in working tree.
- Update history/lifecycle writes to append `events.jsonl`. Done in working tree.
- Update review-thread commands to write per-thread YAML plus Markdown bodies. Done in working tree.
- Update artifact writes to use `artifacts/manifest.yaml` and `artifacts/files/`. Done in working tree for the current UTF-8 `TaskArtifact` API.
- Maintain generated indexes for status, terminal month, relations, and tags. First registry-backed slice implemented in working tree.
- Replace current O(N x files-per-task) v2 list/search scans with generated indexes. Implemented for status, priority, and tag prefiltering; full-text search still scans matched bundle content until Phase 5.
- Move task-lock keying to workspace binding plus canonical task IDs. Implemented in working tree for reservation scope metadata; file-overlap conflict semantics are preserved.
- Preserve lifecycle validation rules, including execution summary before review.
- Decide and implement v2 delete semantics. Implemented in working tree as canonical bundle removal plus projection and registry/index cleanup.

Exit criteria:

- Existing task CLI and tool-host workflows pass against v2 storage.
- Status and terminal listing ergonomics are restored through generated views.
- Relation queries do not scan every bundle.

### Phase 4 Progress

Implemented in working tree:

- Registry schema version 2 with `task_bundle_index`, `task_bundle_tags`, and `task_bundle_relations`.
- Transactional replacement of generated rows from a validated v2 task envelope.
- Workspace-scoped index coverage and `updated_at` version checks so v2 list/filter paths lazily rebuild stale generated rows or fall back to registered bundles.
- V2 list/filter/tag paths read through generated status, priority, and tag indexes when coverage is complete.
- Forward and inverse relation lookup helpers over `task_bundle_relations`.
- Mutation hooks for create, document update, history/status update, review-thread update, and artifact update to refresh generated index rows.
- Task reservation records can carry the v2 `workspace_id`; runtime lock tools pass it from `.orbit/config.yaml`, fail loudly if the v2 binding config disappears, and reservation queries prefer that binding while still seeing legacy path-scoped rows for cleanup.
- V2 delete unregisters task binding/index/relation rows before deleting the canonical home bundle and `.orbit/tasks/<task-id>` projection.
- Review-thread rewrites tombstone removed thread IDs before pruning orphan files so partial rewrites do not resurrect deleted threads.

Still open in Phase 4:

- Use relation indexes in public lineage/query surfaces once those surfaces expose typed v2 relations.
- Multi-file bundle mutations are still detect-and-repair rather than fully transactional; the spec now names the accepted crash midpoints.

## Phase 5 - Consumers And Search

**Status:** In progress

Goal: update readers outside the core task store.

Work:

- Update lexical search to index title, document sidecars, comments, review threads, external refs, and selected text artifacts.
- Update semantic search field names to v2 document names.
- Update web/API task JSON serialization to expose logical v2 fields.
- Update dashboard/status views to use generated indexes.
- Update task locks UI/output to show canonical `ORB-*` IDs.
- Update docs that still describe task sync over the old status-directory shape.

Exit criteria:

- CLI, MCP/tool-host JSON, and web API agree on task field names.
- Search returns snippets from the correct v2 field.
- No consumer depends on old task directory paths.

### Phase 5 Progress

Implemented in working tree:

- V2 lexical search now matches review-thread messages, review-thread paths, external-ref systems, text artifact paths, and text artifact contents in addition to envelope fields, Markdown documents, acceptance criteria, comments, and external-ref IDs.
- Semantic task embedding field names now use v2 logical document names: `title`, `description`, `acceptance`, `plan`, and `execution_summary`.
- Semantic reindex sweeps legacy field rows such as `purpose`, `summary`, and `acceptance_criteria` for each task before writing the renamed v2 fields.
- V2 artifact search skips binary media types and invalid UTF-8 artifact blobs instead of failing the whole task search.
- The `orbit.semantic.search` tool schema and seeded `orbit-semantic` skill now advertise v2 task field names.

Still open in Phase 5:

- Add generated full-text indexes/snippets instead of per-query artifact reads.
- Add semantic parity for review-thread paths/authors, external refs, artifact paths, and artifact text, or explicitly keep those fields lexical-only.
- Update the semantic-search design docs that still describe the pre-v2 field names.
- Decide whether `execution-summary.md` versus logical field `execution_summary` needs an ADR or glossary row.
- Audit web/API/dashboard consumers for old task directory assumptions.
- Update any remaining user-facing docs that describe task sync over old status directories.

## Phase 6 - Remove Old Store Shape

**Status:** In progress

Goal: delete the old task artifact assumptions once v2 is working.

Work:

- Remove `T<YYYYMMDD>-<N>` allocation and validation from task code.
- Remove old status-directory layout code.
- Remove terminal month directory partitioning.
- Remove old YAML fields from task persistence.
- Remove legacy proposed/friction task migrations tied to old task bundles.
- Remove text-only artifact storage assumptions.
- Prune the transitional "current implementation" section from `2_design.md`.
- Flip task-artifacts ADRs from `Proposed` to `Accepted` when the implementation lands.

Exit criteria:

- `make fmt`, `make build`, and `make ci` pass.
- Repository docs describe the live implementation, not a target state.
- Task artifacts are usable enough that future work can be tracked in Orbit itself.

### Phase 6 Progress

Landed:

- **Config gate removed** (`e9582eba`). V2 is the only backend. `[task] artifact_store = "v2"` is accepted as a no-op and `"legacy"` is rejected with a migration error. `TaskArtifactStoreMode`, the legacy `workspace_task_backends` selector, and the `task_artifact_store()` accessors are gone from runtime/builder/context.
- **Legacy store deleted** (`222f6020`). Removed `crates/orbit-store/src/file/task_store/{api,bundle,doc,lock,artifacts,constants,layout,type_migration}.rs`, `crates/orbit-store/src/sqlite/task_index.rs` and the `task_tags` SQL table, the legacy `TaskFileStore` trait impls in `backend/file_backends.rs`, the `workspace_task_backends` factory, the `migrate_legacy_friction_tasks` helper and `FrictionMigrationSummary` type in `friction_store.rs`, the `orbit-core` `command/friction.rs` + `command/migration.rs` runtime entry points, and the `orbit migrate {frictions,task-types}` CLI subcommands plus their tests and audit middleware arms.
- **Design docs pruned and 5 ADRs accepted** (`123f89f7`). Deleted the "Current Implementation" section from `2_design.md`, dropped current/v2 dual framing from `1_overview.md`, and flipped ADR-001 (ORB-00000 IDs), ADR-002 (Envelope + Markdown sidecars), ADR-003 (Status-neutral task directories), ADR-004 (Append-heavy data leaves task.yaml), and ADR-007 (Home task store + symlink projection) to `Accepted` with implementing-commit citations.
- **`workspace_path`/`repo_root` removed from the task update path** (`6beb14a2`). `TaskAutomationUpdate`, `TaskRecordUpdateParams`, and `TaskDocumentUpdateParams` no longer carry these fields; the v2 store's rejection arms for them are gone; worktree/parallel callers and the parallel-timeout mock no longer write them. `Task.workspace_path` and `Task.repo_root` remain on the public DTO (projected at read time from workspace metadata) pending the DTO surgery slice.
- **Binary-capable artifact lexical search** (`6e7e9e4c`). `get_task_artifacts` gates each manifest entry on `is_text_artifact_media_type` and bytes-then-validate UTF-8 (logging and skipping on failure), so a non-text artifact no longer poisons `search_tasks`. Regression test: `search_tasks_skips_binary_artifacts_without_poisoning_results`.

Still open in Phase 6:

- Route `parent_id`, `batch_id`, `dependencies`, and `source_task_id` through the v2 relations API on both create and update; v2 currently hard-errors when these arrive as envelope fields. `batch_id` has no current home in `TaskRelationType` and needs an ADR-005 design decision (new variant vs. envelope attribute vs. tag-like field). Re-enables nine ignored tests (3 in `v2_host/backlog_exclusion_tests` for parent_id, 2 in `task_tools_tests` for dependencies, 1 in `engine/task_host` for batch_id+restamp, 1 for worktree_setup_admits_unplanned_workflow_statuses where batch_id is the remaining blocker, plus 2 dispatch/agent-model tests) and lets ADR-005 flip to `Accepted`.
- Drop the legacy `Task` DTO fields that have no v2 home: `workspace_path`, `repo_root`, `comments`, `history`, `review_threads` (and the `agent`/`model` execution-routing fields if they stay ephemeral). Audit CLI/MCP/engine/web for callers and re-wire to v2 trait methods. This is the biggest blast radius slice; expect a multi-crate ripple. Closing this enables ADR-006 (binary-capable artifact manifest) to flip to `Accepted` once the public `TaskArtifact` DTO carries bytes + media type.

## Status Board

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 0 - Prep And Guardrails | Implemented in working tree | Inventory recorded and v2 fixtures parse under test. |
| Phase 1 - V2 Domain Types | Implemented in working tree | `orbit-common` domain contracts and focused tests are in place. |
| Phase 2 - Home Registry And Workspace Binding | Implemented in working tree | `orbit-store` registry foundation and projection rebuild tests are in place. |
| Phase 3 - V2 Bundle Store | Implemented in working tree | V2 create/get/list/update/review/artifact backend is wired behind `[task] artifact_store = "v2"`. |
| Phase 4 - Task Operations And Local Indexes | In progress | Generated indexes, lock rekeying, relation query acceleration, delete semantics, and review-found repair guards are implemented; public relation query surfaces remain. |
| Phase 5 - Consumers And Search | In progress | First search slice covers v2 review threads/artifacts and semantic field names; consumer audit remains. |
| Phase 6 - Remove Old Store Shape | In progress | Config gate, legacy task_store files, legacy migration commands, runtime `workspace_path`/`repo_root` cleanup, lexical artifact binary-skip, and 5 ADRs landed. Remaining: v2 relations API (route parent_id/batch_id/dependencies/source_task_id; ADR-005 needs a `batch_id` design call), DTO surgery (binary-capable `TaskArtifact`, drop legacy `Task` fields → ADR-006). |

## Latest Validation

2026-05-11:

- `cargo fmt -p orbit-common`
- `cargo test -p orbit-common`
- `cargo fmt -p orbit-store`
- `cargo test -p orbit-store`
- `cargo test -p orbit-store v2_bundle`
- `cargo test -p orbit-store v2_store`
- `cargo test -p orbit-core task_artifact_store`
- `cargo test -p orbit-core v2_task_backend`
- `cargo test -p orbit-store`
- `cargo test -p orbit-core`
- `cargo test -p orbit-embed`
- `cargo test -p orbit-tools`
- `git diff --check`

## Suggested Next Slice

Replace Phase 5's per-query artifact reads with generated full-text search rows and snippets, then audit web/API/dashboard consumers for old status-directory assumptions. Decide whether semantic search should index review-thread paths/authors, external refs, and artifact text or keep those as lexical-only fields.
