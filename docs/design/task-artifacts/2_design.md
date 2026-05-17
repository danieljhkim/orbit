# Task Artifacts — Design

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-17

This document describes the v2 task artifact implementation. The v2 store is the only task backend; the legacy status-directory store and its `[task] artifact_store` config gate were removed once Phase 6 began (`e9582eba`), and stale `artifact_store` keys are now rejected for every value. Sections below are prescriptive about invariants the live store maintains rather than aspirational about a target.

---

## 1. Bundle Layout

The v2 task bundle is status-neutral. The canonical bundle lives in the user's local Orbit home, partitioned by workspace identity:

```text
~/.orbit/tasks/
  index.sqlite
  workspaces/
    orbit-a3f9c2/
      ORB-00000/
        task.yaml
        description.md
        acceptance.md
        plan.md
        execution-summary.md
        events.jsonl
        comments.jsonl
        review-threads/
          RT-0001.yaml
          RT-0001.md
        artifacts/
          manifest.yaml
          files/
```

Each checkout has a small workspace binding and symlink projection:

```text
.orbit/
  config.yaml
  tasks/
    ORB-00000 -> ~/.orbit/tasks/workspaces/orbit-a3f9c2/ORB-00000
```

Status lives in `task.yaml`. Directory moves are not part of lifecycle transitions. Read-side indexes or generated views present status groupings, terminal-month views, and dashboard counts for humans.

The canonical task directory is outside the checkout. `.orbit/tasks/` should remain ignored by Git and treated as a projection that Orbit can rebuild. ADRs and design docs remain committed project memory.

## 2. Envelope Schema

`task.yaml` should be small and structured:

```yaml
schema_version: 1
id: ORB-00001
title: Reset task artifact schema
status: proposed
type: feature
priority: high
complexity: hard
job_run_id: null
relations:
  - type: supersedes
    target: ORB-00000
  - type: resolves
    target: F2026-05-007
context_files:
  - file:crates/orbit-store/src/file/task_store/v2_bundle.rs
external_refs: []
created_by: codex:gpt-5.5
planned_by: null
implemented_by: null
created_at: 2026-05-11T00:00:00Z
updated_at: 2026-05-11T00:00:00Z
```

The envelope should not include prose bodies, comments, review message bodies, execution summaries, `workspace_path`, `repo_root`, `agent`, `model`, or the old `batch_id` name. Local execution bindings live in the local task registry, keyed by task ID and workspace binding. Execution fan-out membership is `job_run_id`, a foreign reference to the job-run store rather than a task relation. `relations` can also carry cross-artifact provenance via `produces` and `resolves` targets (`ORB-`, `F`, `L`, or `ADR-` IDs); legacy task relation types remain task-only. `schema_version` restarts at `1` because the reset defines a new artifact family; the cutover command knows how to read the old schema but v2 does not continue its compatibility stream.

## 3. Prose Documents

The v2 store treats these sidecars as first-class documents:

| File | Document name | Writer |
|------|---------------|--------|
| `description.md` | `description` | Task author or planning agent |
| `acceptance.md` | `acceptance` | Task author, planning agent, reviewer |
| `plan.md` | `plan` | Planner or executing agent |
| `execution-summary.md` | `execution-summary` | Implementing agent |

`acceptance.md` should prefer Markdown task list syntax:

```markdown
- [ ] Behavior X is implemented.
- [ ] Regression Y is covered.
- [ ] `make ci` passes.
```

APIs should treat the Markdown file as the source of truth. They may offer parsed checkbox views for UI convenience, but v2 storage should not preserve the old YAML array as a compatibility contract.

## 4. Append-only Logs

`events.jsonl` records lifecycle and metadata events. Each row includes at least:

```json
{"schema_version":1,"event_id":"EV-0001","at":"2026-05-11T00:00:00Z","by":"codex:gpt-5.5","type":"created","from_status":null,"to_status":"proposed","note":null}
```

`comments.jsonl` records task comments:

```json
{"schema_version":1,"comment_id":"C-0001","at":"2026-05-11T00:00:00Z","by":"daniel","body":"Start over from the schema reset."}
```

Review threads get one metadata envelope and one Markdown body per thread. This keeps thread state structured while making review prose easy to read and diff.

JSONL appends use a single-line JSON record, append mode, flush, and file sync before success. Readers tolerate a final unterminated or invalid line as tail corruption: valid preceding rows remain readable, and repair may truncate only the corrupt tail. Sidecar rewrites and review-thread YAML updates use write-temp-in-same-directory, file sync, atomic rename, and parent-directory sync.

V2 does not provide cross-file transactions. A crash can leave an appended event before the envelope status update, artifact files before the manifest update, or updated review-thread files before stale thread files are removed. The store must keep every intermediate state readable and recoverable, but generated repair/indexing passes are responsible for reconciling partial multi-file mutations.

## 5. Artifact Manifest

Artifacts support text and binary files. The public `TaskArtifact` DTO carries `path`, raw byte `content`, and `media_type`; the bundle persists those bytes under `artifacts/files/` and indexes them with `artifacts/manifest.yaml`:

```yaml
schema_version: 1
files:
  - path: planning-duel/winner.json
    blob: files/planning-duel/winner.json
    media_type: application/json
    sha256: "<64 lowercase hex chars>"
    created_by: codex:gpt-5.5
```

Manifest paths are stored in canonical relative form: slash-separated, no absolute paths, no `..`, no `.`, and no leading `./`. Writers that ingest hand-authored manifests should normalize a leading `./` before validation. SHA-256 values are lowercase hex; writer code should format digest bytes with lowercase hex (`{:x}`).

Text artifacts may still be rendered inline by `orbit.task.show --field artifacts`, but storage and API DTOs must not require UTF-8.

## 6. Local Task Store and Symlink Projection

Local-first Orbit uses `~/.orbit/tasks/` as the canonical store for task artifacts. `index.sqlite` owns allocation and local operational metadata; `workspaces/<workspace-id>/` owns the actual bundles:

```text
~/.orbit/tasks/
  index.sqlite
  workspaces/
    orbit-a3f9c2/
      ORB-00000/
    my-app-b7e1d8/
      ORB-00023/
```

`index.sqlite` tracks:

- the machine-local allocation authority and its next available numeric suffix;
- workspace bindings, including `workspace_id`, slug, repo root, workspace path, `.orbit` path, and optional remote/path fingerprints for rebind;
- task-to-workspace bindings for resolving local operations such as `orbit task start`;
- generated status, terminal-month, relation, and tag indexes for fast list/filter/relation surfaces;
- task-lock reservations or pointers to the existing `task_reservations` SQLite store, keyed by workspace binding and canonical task IDs while preserving file-overlap conflict checks.

`.orbit/config.yaml` stores the checkout binding:

```yaml
schema_version: 1
workspace_id: orbit-a3f9c2
```

`workspace_id` is assigned once as `<slug>-<6char>`, where the slug is human-readable and the suffix prevents collisions. It survives repo renames and moves because Orbit reads it from `.orbit/config.yaml`, not from the directory name, remote URL, or path hash.

The workspace projection under `.orbit/tasks/<task-id>` is a symlink to the canonical bundle. Task writes through either the canonical path or the projection update the same files; there is no second writable copy and no bundle-level divergence protocol. If `.orbit/tasks/` is deleted, Orbit rebuilds the symlinks from `.orbit/config.yaml` and `index.sqlite`. If `.orbit/config.yaml` is lost, Orbit prompts to rebind by matching the current path, repo root, and optional remote fingerprints against `index.sqlite`; if no confident match exists, the user chooses or creates a workspace binding.

Task delete first verifies that any projection entry is a symlink, then unregisters the binding from `index.sqlite`, deletes the canonical home bundle, and removes the projection. Generated index rows and relation edges involving the deleted task are removed with the binding. If deletion is interrupted after unregistering, the remaining bundle is orphaned storage rather than a listed task and a retry may finish cleanup.

## 7. Generated Local Indexes

The bundle remains canonical. The registry maintains generated projections from each task envelope:

- `task_bundle_index`: one row per registered task with workspace, status, priority, `job_run_id`, timestamps, and terminal month.
- `task_bundle_tags`: normalized tag rows with AND-style filtering semantics.
- `task_bundle_relations`: directed `(source_task_id, relation_type, target_id)` rows plus an inverse lookup index for task targets.

Task mutations rewrite the generated rows after the envelope write. The index row `updated_at` is a version stamp for the canonical envelope. V2 list and filter paths may use the index only when every registered task has an index row and every indexed `updated_at` matches the bundle envelope. Count or version mismatches trigger a lazy rebuild from registered bundles; if rebuild fails, queries fall back to reading bundles directly. Full-text search still scans task content until the Phase 5 lexical/semantic indexes land.

## 8. Crash Consistency

The v2 bundle is local and file-backed, so multi-file mutations are not fully transactional. The implementation keeps the envelope canonical and makes generated data rebuildable, but the following interrupted states are expected repair cases:

- Document updates may write Markdown sidecars before `task.yaml`; readers return the sidecar content and the previous envelope metadata until the next successful mutation.
- History updates may append `events.jsonl` before `task.yaml`; readers reject bundles when the last status event does not match the envelope status.
- Artifact updates may write files before `manifest.yaml`; unreferenced files under `artifacts/files/` are ignored, while manifest entries with missing files, size drift, or hash drift are corruption and fail loudly.
- Review-thread rewrites tombstone removed thread IDs before pruning files, so a crash before prune does not resurrect deleted threads.
- Generated index writes may fail after the envelope changes; `updated_at` validation detects the stale row and rebuilds from bundles before indexed reads.

Task lock reservations in v2 mode require `.orbit/config.yaml` to provide the workspace binding. If that file disappears while a runtime is active, lock writes fail instead of silently creating legacy `NULL`-workspace reservations.

---

## 9. ID Allocation

### 9.1 Format

The target canonical ID format is:

```text
ORB-00000
```

`ORB` names the product namespace. The suffix is decimal, zero-padded to five digits for the initial range (`ORB-00000` through `ORB-99999`). The string is the task identity inside one allocation authority. Consumers may parse the numeric suffix only for validation and ordering.

The local OSS authority is one `~/.orbit/tasks/index.sqlite` allocator shared across all local workspaces, so a single machine will not mint the same `ORB-00042` for two repositories. Bare IDs are still not universally unique across unrelated machines or hosted tenants. Cross-registry references must carry registry/workspace context through the sync registry, hosted tenant, or an explicit external reference. Code should not infer universal uniqueness from the `ORB-` prefix alone.

### 9.2 Flat storage

Canonical task bundles live under `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/`; projected workspace paths live at `.orbit/tasks/<task-id>`. The initial design deliberately avoids numeric partition directories. Expected local and small-team task counts do not justify the extra path complexity, and a later ADR can add fanout with a migration if a real corpus hits filesystem limits.

### 9.3 Allocation authority

Uniqueness requires an authority:

- Local-only OSS allocates from one machine-local registry under `~/.orbit/tasks/index.sqlite`, guarded by a process/file lock.
- Synced workspaces allocate against the task registry before materializing a bundle.
- Hosted team mode allocates through the hosted API.

The implementation should not claim authority-scoped uniqueness by scanning one workspace's `.orbit/tasks/` tree. Allocation reserves an ID. Workspace binding, symlink projection, sync upload, and hosted publication are separate APIs.

### 9.4 No legacy aliases

V2 task bundles do not carry `legacy_ids`, and lookup surfaces should not resolve old `T<YYYYMMDD>-<N>` values. During a one-time cutover, Orbit may print a local report that maps old IDs to new IDs, but that report is an operator aid rather than part of the task schema. New commits and docs should cite only `ORB-00000` IDs.

---

## 10. API Contract

The public task API should expose the v2 bundle model directly:

- envelope metadata from `task.yaml`;
- Markdown documents by document name (`description`, `acceptance`, `plan`, `execution-summary`);
- append-only streams for events and comments;
- review-thread metadata plus Markdown message bodies;
- artifact manifest entries.

CLI and tool selectors may keep friendly names such as `description`, `plan`, and `execution-summary`, but they should be implemented as first-class document reads and writes. New internal code should operate on a bundle abstraction with explicit envelope, document, log, thread, and artifact fields.

The public `Task` DTO should not embed legacy relation fields (`parent_id`, `dependencies`, `source_task_id`), local workspace bindings (`workspace_path`, `repo_root`), append-heavy streams (`comments`, `history`, `review_threads`), or internal execution-routing fields (`agent`, `model`). It carries typed `relations`, `job_run_id`, envelope metadata, and durable attribution fields. Consumers that need comments, history, review threads, or workspace binding metadata should call the dedicated bundle/registry APIs.

`orbit.task.locks.*` remains a local operational surface, not a task artifact. Lock reservations live in SQLite keyed by workspace binding and canonical `ORB-*` task IDs, and expire by TTL.

---

## 11. Search and Indexing

Lexical and semantic search should index each logical field independently:

- `title`
- `description`
- `acceptance`
- `plan`
- `execution_summary`
- `comments`
- `review_threads` (message bodies, message authors, and paths)
- `external_refs` (system and id)
- artifact paths plus selected artifact text, when media type permits

This preserves field-aware semantic search while making file boundaries visible in snippets. The embedding index should store field names that match the v2 logical document names; `execution-summary.md` is exposed as `execution_summary` to match the tool/API field.

The current Phase 5 implementation is intentionally asymmetric while indexes are still being wired. Lexical search scans the broader set above. Semantic search indexes task title, description, acceptance, plan, and execution summary; semantic parity for comments, review messages and paths/authors, external refs, and artifacts remains Phase 5 follow-up work.

Until generated full-text indexes land, the working implementation performs O(N x files-per-task) lexical scans by reading every registered bundle and any candidate text artifact files. That is acceptable only as a cutover bridge; generated search rows will replace the per-query artifact reads.

Relations need their own generated index. The bundle stores directed relation entries; local indexes materialize `(source_task_id, relation_type, target_task_id)` and optional inverse views for efficient lineage queries. The initial relation type set is `blocked_by`, `child_of`, `spawned_from`, `regression_from`, `supersedes`, and `related_to`. Types are source-implied: a task that depends on another stores `blocked_by -> dependency`, and a subtask stores `child_of -> parent`. Writers validate relation types, reject self-edges and duplicates, and reject cycles for hierarchy and blocking relation families.

Status and retention views are also generated indexes. Terminal tasks remain in `.orbit/tasks/<task-id>/`, but CLI/list surfaces can group by terminal month using status-transition events. Compaction is out of scope for the reset, but the index must preserve the old ergonomic affordance of listing active tasks and closed tasks separately.

---

## 12. Cutover Path

The reset should be a one-time cutover rather than a long-lived compatibility layer:

1. Create or reuse a `workspace_id` in `.orbit/config.yaml`.
2. Allocate or derive a canonical `ORB-00000` ID for every existing task.
3. Record the authority allocation and workspace binding in `~/.orbit/tasks/index.sqlite`.
4. Materialize each task into `~/.orbit/tasks/workspaces/<workspace-id>/<canonical-id>/`.
5. Create `.orbit/tasks/<canonical-id>` as a symlink to the canonical bundle.
6. Move `description` from `task.yaml` to `description.md`.
7. Render `acceptance_criteria` into `acceptance.md`.
8. Keep existing `plan.md` and `execution-summary.md`.
9. Move `history` into `events.jsonl`.
10. Move `comments` into `comments.jsonl`.
11. Move `review_threads` into `review-threads/`.
12. Rewrite `task.yaml` as the v2 envelope without old IDs or embedded prose.
13. Rewrite task-lock reservations from old IDs to canonical IDs or release stale reservations with an audit event.
14. Rebuild task tag, semantic, status, terminal-month, and relation indexes from disk.

The cutover command may emit a human-readable mapping from old IDs to new IDs, but Orbit should not persist that mapping as a lookup surface.

---

## 13. Concerns & Honest Limitations

`ORB-00000` IDs look universal but are only unique inside an allocation authority. Local-only Orbit uses one machine-local authority across all repos, but sync and hosted modes need shared allocation. That is a deliberate tradeoff: a context-free global ID cannot be produced across machines without a registry.

Markdown acceptance criteria are friendlier to authors but weaker than typed checks. Automation gates that require structured checks should add a future `checks.yaml` instead of keeping the old YAML array alive.

Status-neutral directories simplify sync but make filesystem browsing less convenient. Humans lose the quick `ls .orbit/tasks/review` view unless Orbit generates indexes or CLI views. The reset must ship those generated views for active/status and terminal-month grouping, because lifecycle state belongs in the record but humans still need fast browsing.

Append-only logs improve merge behavior but add more files per task. Small tasks become slightly noisier on disk. The benefit is that high-traffic tasks stop rewriting one large YAML file for every comment or event.

Binary artifacts increase storage flexibility and require stronger validation. Checksums, media type inference, size limits, and redaction rules become part of the artifact contract instead of being avoided by UTF-8-only writes.

`.orbit/config.yaml` becomes load-bearing for binding a checkout to its canonical task store. If it is lost, Orbit can try to rebind by path and repo fingerprints, but ambiguous matches require a user decision. Symlink projection also needs a fallback on platforms or filesystems where symlink creation is restricted.

---

## Task References

- [T20260505-12] — Designed git-orphan-branch task sync and documented the current sync-era task bundle assumptions.
- [T20260506-11] — Removed knowledge-graph task attribution and documented why old task IDs were only local search keys.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
