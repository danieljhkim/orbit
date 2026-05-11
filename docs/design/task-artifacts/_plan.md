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

Implementation has started. Phase 0 and Phase 1 landed in `c1f72a32`. Phase 2 landed in `1ae83804`. Phase 3 is in progress: the first working-tree slice adds v2 bundle read/write primitives, durable sidecar writes, JSONL append/tail repair, registry registration, and projection rebuild support. These primitives compile and pass module-local tests, but no live task backend consumes them yet; `task_store/mod.rs` carries a temporary `#[allow(dead_code)]` until the next slice wires them into `task add`, `task show`, and `task list`.

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

**Status:** In progress

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
- Durable creation for `task.yaml`, Markdown sidecars, `events.jsonl`, `comments.jsonl`, `review-threads/`, and `artifacts/files/`.
- Registry-backed canonical bundle paths, task-bundle registration, and workspace projection rebuild after bundle creation.
- Per-task create locking so same-ID bundle creation serializes before the existence check.
- Projection rebuild failures after registration return degraded success rather than exposing a failed create for committed state.
- Durable document rewrites through `atomic_write_text`.
- JSONL append helpers with sibling file lock, tail repair, flush, and file sync.
- Tests for round-trip storage, review-thread files, registry-backed listing, document rewrites, append behavior, tail repair, corruption rejection before the tail, concurrent appends, rename invariant rejection, projection degraded success, and cleanup after failed creation.

## Phase 4 - Task Operations And Local Indexes

**Status:** Not started

Goal: restore the full task workflow on top of v2 storage.

Work:

- Update document updates to write Markdown sidecars.
- Update comments to append `comments.jsonl`.
- Update history/lifecycle writes to append `events.jsonl`.
- Update review-thread commands to write per-thread YAML plus Markdown bodies.
- Update artifact writes to use `artifacts/manifest.yaml` and `artifacts/files/`.
- Maintain generated indexes for status, terminal month, relations, and tags.
- Move task-lock keying to workspace binding plus canonical task IDs.
- Preserve lifecycle validation rules, including execution summary before review.

Exit criteria:

- Existing task CLI and tool-host workflows pass against v2 storage.
- Status and terminal listing ergonomics are restored through generated views.
- Relation queries do not scan every bundle.

## Phase 5 - Consumers And Search

**Status:** Not started

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

## Phase 6 - Remove Old Store Shape

**Status:** Not started

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

## Status Board

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 0 - Prep And Guardrails | Implemented in working tree | Inventory recorded and v2 fixtures parse under test. |
| Phase 1 - V2 Domain Types | Implemented in working tree | `orbit-common` domain contracts and focused tests are in place. |
| Phase 2 - Home Registry And Workspace Binding | Implemented in working tree | `orbit-store` registry foundation and projection rebuild tests are in place. |
| Phase 3 - V2 Bundle Store | In progress | V2 bundle primitives are in working tree behind temporary dead-code allowance; CLI/store wiring remains. |
| Phase 4 - Task Operations And Local Indexes | Not started | Restores workflow completeness. |
| Phase 5 - Consumers And Search | Not started | Updates surfaces outside storage. |
| Phase 6 - Remove Old Store Shape | Not started | Cleanup after v2 passes. |

## Latest Validation

2026-05-11:

- `cargo fmt -p orbit-common`
- `cargo test -p orbit-common`
- `cargo fmt -p orbit-store`
- `cargo test -p orbit-store`
- `cargo test -p orbit-store v2_bundle`

## Suggested Next Slice

Wire the v2 bundle store into the task backend constructor path, carrying the home task registry and workspace binding instead of hiding v2 behind the current status-directory root. The smallest useful surface is `create_task`, `get_task`, and `list_tasks`; document/history/review/artifact updates can follow once that spine works.
