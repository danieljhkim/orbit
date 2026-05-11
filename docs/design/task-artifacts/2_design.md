# Task Artifacts — Design

**Status:** Draft
**Owner:** codex
**Last updated:** 2026-05-11

This document describes the current task artifact implementation and the target v2 shape. The current implementation is still the source of truth until a cutover task lands. The v2 sections are intentionally prescriptive: they define the schema direction the next implementation should converge on instead of preserving every historical convenience. This is a transitional document: when the cutover ships, the implementing change should prune or archive the current-implementation section so `2_design.md` again describes only the live implementation.

---

## 1. Current Implementation

### 1.1 Storage layout

The current store is file-backed under `.orbit/tasks/`. Non-terminal statuses live directly under status directories:

```text
.orbit/tasks/
  proposed/<task-id>/
  backlog/<task-id>/
  someday/<task-id>/
  in_progress/<task-id>/
  review/<task-id>/
  blocked/<task-id>/
```

Terminal statuses are date-partitioned:

```text
.orbit/tasks/
  done/<yyyy-mm>/<task-id>/
  archived/<yyyy-mm>/<task-id>/
  rejected/<yyyy-mm>/<task-id>/
```

Each task directory contains:

- `task.yaml`
- `plan.md`
- `execution-summary.md`
- `artifacts/`

This is implemented in [crates/orbit-store/src/file/task_store/layout.rs](../../../crates/orbit-store/src/file/task_store/layout.rs) and [constants.rs](../../../crates/orbit-store/src/file/task_store/constants.rs).

### 1.2 Current YAML envelope

`task.yaml` currently carries both metadata and content. The persisted document includes:

- Identity: `id`, `parent_id`, `type`, `priority`, `complexity`.
- Content: `title`, `description`, `acceptance_criteria`, `dependencies`, `tags`.
- Context: `context_files`, `workspace_path`, `repo_root`.
- Ownership: `created_by`, `planned_by`, `implemented_by`.
- Implementation metadata: `model`, `pr_status`, `external_refs`.
- Attribution: `source_task_id`, `batch_id`.
- Timestamps: `created_at`, `updated_at`.
- Audit arrays: `history`, `comments`, `review_threads`.

The store splits only `plan` and `execution_summary` into Markdown companions. On read, [bundle.rs](../../../crates/orbit-store/src/file/task_store/bundle.rs) reconstructs a flat `Task` with string fields for `description`, `plan`, and `execution_summary`.

### 1.3 Current ID allocation

Current IDs have the form `T<YYYYMMDD>-<N>`, with optional legacy numeric suffixes. Allocation scans task directories for the current date and increments the highest suffix. The local task allocation lock serializes concurrent writers on one machine, but the allocator has no cross-workspace authority.

The validator rejects any other ID shape. That makes the format deeply embedded in lookup, lock, update, artifact, and delete entry points.

### 1.4 Current mutation model

Task mutations read a full bundle, update in memory, and rewrite the bundle. Lifecycle transitions also move the task directory between status paths. Comments, history entries, and review threads are YAML arrays, so append-only operations rewrite `task.yaml`.

This model is simple and inspectable on one machine. It becomes awkward under sync because directory moves and YAML-array appends create avoidable conflicts.

---

## 2. Target V2 Artifact Shape

### 2.1 Layout

The target v2 task bundle is status-neutral. The canonical bundle lives in the user's local Orbit home, partitioned by workspace identity:

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

### 2.2 Envelope schema

`task.yaml` should be small and structured:

```yaml
schema_version: 1
id: ORB-00001
title: Reset task artifact schema
status: proposed
type: feature
priority: high
complexity: hard
relations:
  - type: supersedes
    target: ORB-00000
context_files:
  - file:crates/orbit-store/src/file/task_store/doc.rs
external_refs: []
created_by: codex:gpt-5.5
planned_by: null
implemented_by: null
created_at: 2026-05-11T00:00:00Z
updated_at: 2026-05-11T00:00:00Z
```

The envelope should not include prose bodies, comments, review message bodies, execution summaries, `workspace_path`, or `repo_root`. Local execution bindings live in the local task registry, keyed by task ID and workspace binding. `schema_version` restarts at `1` because the reset defines a new artifact family; the cutover command knows how to read the old schema but v2 does not continue its compatibility stream.

### 2.3 Prose documents

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

### 2.4 Append-only logs

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

### 2.5 Artifact manifest

Artifacts should support text and binary files. The current `TaskArtifact { path, content }` model only accepts UTF-8 text. V2 artifacts should use `artifacts/manifest.yaml`:

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

Text artifacts may still be rendered inline by `orbit.task.show --field artifacts`, but storage should not require UTF-8.

### 2.6 Local task store and symlink projection

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
- generated status, terminal-month, relation, and tag indexes for fast list/search surfaces;
- task-lock reservations or pointers to the existing `task_reservations` SQLite store, keyed by workspace binding and canonical task IDs.

`.orbit/config.yaml` stores the checkout binding:

```yaml
schema_version: 1
workspace_id: orbit-a3f9c2
```

`workspace_id` is assigned once as `<slug>-<6char>`, where the slug is human-readable and the suffix prevents collisions. It survives repo renames and moves because Orbit reads it from `.orbit/config.yaml`, not from the directory name, remote URL, or path hash.

During the cutover window, the runtime selects this store only when workspace `config.toml` sets `[task] artifact_store = "v2"`. That gate is transitional: once v2 covers indexes, locks, and delete semantics, the legacy status-directory store should be removed rather than preserved as a long-term compatibility mode.

The workspace projection under `.orbit/tasks/<task-id>` is a symlink to the canonical bundle. Task writes through either the canonical path or the projection update the same files; there is no second writable copy and no bundle-level divergence protocol. If `.orbit/tasks/` is deleted, Orbit rebuilds the symlinks from `.orbit/config.yaml` and `index.sqlite`. If `.orbit/config.yaml` is lost, Orbit prompts to rebind by matching the current path, repo root, and optional remote fingerprints against `index.sqlite`; if no confident match exists, the user chooses or creates a workspace binding.

---

## 3. ID Allocation

### 3.1 Format

The target canonical ID format is:

```text
ORB-00000
```

`ORB` names the product namespace. The suffix is decimal, zero-padded to five digits for the initial range (`ORB-00000` through `ORB-99999`). The string is the task identity inside one allocation authority. Consumers may parse the numeric suffix only for validation and ordering.

The local OSS authority is one `~/.orbit/tasks/index.sqlite` allocator shared across all local workspaces, so a single machine will not mint the same `ORB-00042` for two repositories. Bare IDs are still not universally unique across unrelated machines or hosted tenants. Cross-registry references must carry registry/workspace context through the sync registry, hosted tenant, or an explicit external reference. Code should not infer universal uniqueness from the `ORB-` prefix alone.

### 3.2 Flat storage

Canonical task bundles live under `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/`; projected workspace paths live at `.orbit/tasks/<task-id>`. The initial design deliberately avoids numeric partition directories. Expected local and small-team task counts do not justify the extra path complexity, and a later ADR can add fanout with a migration if a real corpus hits filesystem limits.

### 3.3 Allocation authority

Uniqueness requires an authority:

- Local-only OSS allocates from one machine-local registry under `~/.orbit/tasks/index.sqlite`, guarded by a process/file lock.
- Synced workspaces allocate against the task registry before materializing a bundle.
- Hosted team mode allocates through the hosted API.

The implementation should not claim authority-scoped uniqueness by scanning one workspace's `.orbit/tasks/` tree. Allocation reserves an ID. Workspace binding, symlink projection, sync upload, and hosted publication are separate APIs.

### 3.4 No legacy aliases

V2 task bundles do not carry `legacy_ids`, and lookup surfaces should not resolve old `T<YYYYMMDD>-<N>` values. During a one-time cutover, Orbit may print a local report that maps old IDs to new IDs, but that report is an operator aid rather than part of the task schema. New commits and docs should cite only `ORB-00000` IDs.

---

## 4. API Contract

The public task API should expose the v2 bundle model directly:

- envelope metadata from `task.yaml`;
- Markdown documents by document name (`description`, `acceptance`, `plan`, `execution-summary`);
- append-only streams for events and comments;
- review-thread metadata plus Markdown message bodies;
- artifact manifest entries.

CLI and tool selectors may keep friendly names such as `description`, `plan`, and `execution-summary`, but they should be implemented as first-class document reads and writes. New internal code should operate on a bundle abstraction with explicit envelope, document, log, thread, and artifact fields.

`orbit.task.locks.*` remains a local operational surface, not a task artifact. Lock reservations live in SQLite keyed by workspace binding and task IDs, expire by TTL, and are rebuilt or released during cutover when old IDs are replaced.

---

## 5. Search and Indexing

Lexical and semantic search should index each logical field independently:

- `title`
- `description`
- `acceptance`
- `plan`
- `execution-summary`
- `comments`
- `review_threads`
- selected artifact text, when media type permits

This preserves field-aware semantic search while making file boundaries visible in snippets. The embedding index should store field names that match the v2 document names.

Until the generated indexes land, the working implementation performs O(N x files-per-task) scans for list/search by reading every registered bundle and its sidecars. That is acceptable only as a cutover bridge; Phase 4 replaces it with generated status, tag, relation, terminal-month, and search inputs.

Relations need their own generated index. The bundle stores directed relation entries; local indexes materialize `(source_task_id, relation_type, target_task_id)` and optional inverse views for efficient lineage queries. The initial relation type set is `blocks`, `parent_of`, `spawned_from`, `regression_from`, `supersedes`, and `related_to`. Writers validate relation types, reject self-edges and duplicates, and reject cycles for hierarchy and blocking relation families. Reciprocal labels such as `blocked_by` are read-side projections, not separately stored peer edges.

Status and retention views are also generated indexes. Terminal tasks remain in `.orbit/tasks/<task-id>/`, but CLI/list surfaces can group by terminal month using status-transition events. Compaction is out of scope for the reset, but the index must preserve the old ergonomic affordance of listing active tasks and closed tasks separately.

---

## 6. Cutover Path

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

## 7. Concerns & Honest Limitations

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
