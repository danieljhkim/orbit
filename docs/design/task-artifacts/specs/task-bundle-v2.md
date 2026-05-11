# Spec: Task Bundle V2

Task Bundle V2 defines the canonical on-disk contract for Orbit tasks after the schema reset. The contract is source-of-truth storage, not merely a rendering preference: implementations should expose the bundle shape below rather than maintaining long-lived compatibility with the previous task schema.

## Why This Exists

The current task bundle is easy to inspect but hard to sync and evolve. Long prose, append-heavy audit arrays, status directory moves, and workspace-local IDs all interact badly with shared registries and long-lived agent workflows. This spec pins the reset target so cutover work and new code converge on one artifact shape.

## Normative Language

`must` marks required behavior for conforming v2 readers and writers. `should` marks a recommended default that improves authoring or UX but is not a storage-level validity rule unless a transition policy explicitly enforces it.

## Bundle Path

Every canonical task bundle lives at:

```text
~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/
```

The workspace-local projection lives at:

```text
.orbit/tasks/<task-id> -> ~/.orbit/tasks/workspaces/<workspace-id>/<task-id>
```

`<task-id>` must be the canonical ID inside the current allocation authority. The canonical v2 format is `ORB-` plus a five-digit decimal suffix (`ORB-00000` through `ORB-99999`). `<workspace-id>` is assigned once per workspace as `<slug>-<6char>` and stored in `.orbit/config.yaml`. Old `T<YYYYMMDD>-<N>` IDs are not valid v2 identifiers or lookup aliases.

Required files:

```text
task.yaml
description.md
acceptance.md
plan.md
execution-summary.md
events.jsonl
comments.jsonl
```

Required directories:

```text
review-threads/
artifacts/
```

`artifacts/manifest.yaml` is required when `artifacts/files/` contains any files.

## Local Store and Workspace Projection

The home-directory bundle is the active source of truth for task content. Local-first Orbit must keep allocation and local operational metadata under:

```text
~/.orbit/tasks/index.sqlite
```

Canonical workspace bundles live under:

```text
~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/
```

The index must record allocation authority state, canonical IDs, workspace bindings, materialized bundle paths, local execution bindings such as `workspace_path` and `repo_root`, generated status/relation/tag index rows, and task-lock reservation bindings or pointers.

Each workspace must have `.orbit/config.yaml` with at least:

```yaml
schema_version: 1
workspace_id: orbit-a3f9c2
```

`.orbit/tasks/` is a symlink projection to canonical bundles. Task mutations must make the canonical bundle and registry metadata durable before reporting success. If `.orbit/tasks/` is deleted, Orbit rebuilds projection links from `.orbit/config.yaml` and `index.sqlite`. If `.orbit/config.yaml` is missing, Orbit must prompt to rebind by matching the checkout path, repo root, and optional remote fingerprints against `index.sqlite`; ambiguous matches must not silently attach to a workspace.

## Envelope

`task.yaml` must contain only structured metadata:

```yaml
schema_version: 1
id: ORB-00000
title: Short title
status: proposed
type: chore
priority: medium
complexity: null
relations: []
tags: []
context_files: []
external_refs: []
created_by: codex:gpt-5.5
planned_by: null
implemented_by: null
created_at: 2026-05-11T00:00:00Z
updated_at: 2026-05-11T00:00:00Z
```

The envelope must not contain `description`, `acceptance_criteria`, `plan`, `execution_summary`, `history`, `comments`, or `review_threads`.
The envelope must not contain local-only `workspace_path` or `repo_root`; those live in the local registry workspace binding.

## Documents

Document sidecars are UTF-8 Markdown:

- `description.md` may be empty only for machine-generated placeholder tasks.
- `acceptance.md` should have at least one bullet or checkbox before a task enters `in-progress`; storage does not define a waiver list.
- `plan.md` must be non-empty before entering `in-progress` when the transition policy requires a plan.
- `execution-summary.md` must be non-empty before entering `review`.

Convenience reads:

- `description` returns `description.md` as a string.
- `acceptance` returns `acceptance.md` as a string; UI code may parse checkboxes for display.
- `plan` returns `plan.md` as a string.
- `execution-summary` returns `execution-summary.md` as a string.

## Events

`events.jsonl` is append-only. Each line is a JSON object with:

- `schema_version`: event row schema version, initially `1`.
- `event_id`: stable per-task event ID.
- `at`: RFC 3339 timestamp.
- `by`: actor label.
- `type`: event type.
- `note`: optional string.
- `from_status`: optional status.
- `to_status`: optional status.

Writers must not rewrite prior event lines except during explicit cutover or repair commands. Appends must write one JSON object plus newline, flush and sync the file before success, and preserve a valid prefix if the process crashes. Readers must tolerate a final unterminated or invalid JSON line as tail corruption; repair may truncate only that final corrupt tail.

## Comments

`comments.jsonl` is append-only. Each line is a JSON object with:

- `schema_version`: comment row schema version, initially `1`.
- `comment_id`
- `at`
- `by`
- `body`

Comment bodies are Markdown strings. Multi-line bodies must be JSON escaped inside one JSONL row, not spread across lines.
Comment appends follow the same atomicity and tail-repair rules as `events.jsonl`.

## Review Threads

Each review thread uses:

```text
review-threads/<thread-id>.yaml
review-threads/<thread-id>.md
```

The YAML file stores status, file path, line/range metadata, external review IDs, timestamps, and message metadata. The Markdown file stores message bodies in chronological order with stable message anchors.
The YAML file must include `schema_version: 1`. YAML rewrites use write-temp-in-same-directory, file sync, atomic rename, and parent-directory sync.
Review-thread rewrites must validate the complete replacement set before writing and must not remove `review-threads/` before the replacement is durable. Crash recovery may leave stale thread files behind, but it must not leave the bundle unreadable solely because the directory vanished.

## Relations

Relations are directed entries stored in the task envelope. The initial relation type set is:

- `blocks`: source task blocks target task.
- `parent_of`: source task owns or contains target task.
- `spawned_from`: source task was created from target task.
- `regression_from`: source task tracks a regression introduced by target task.
- `supersedes`: source task replaces target task.
- `related_to`: source task is associated with target task without stronger semantics.

Writers must validate the relation type set, reject self-edges, reject duplicate `(type, target)` entries on one source task, and reject cycles for hierarchy and blocking relation families. Inverse labels such as `blocked_by` are read-side projections generated from a stored `blocks` edge, not separately stored peer records.

Local indexes should materialize `(source_task_id, relation_type, target_task_id)` and inverse lookup rows so lineage queries do not scan every task bundle.

## Generated Views

The local registry must maintain generated status and terminal-month views or rebuild them from bundles/events on demand. These views replace the current `review/` and `done/<yyyy-mm>/` directory browsing affordances without making lifecycle state part of the path.

## Local Locks

Task lock reservations are not task artifacts. They remain local operational state in SQLite, keyed by workspace binding and canonical task IDs, with TTL/release semantics and audit events. Cutover must rewrite active reservations to canonical IDs or release stale reservations with an audit event.

## Artifacts

Artifacts are stored under `artifacts/files/` and listed in `artifacts/manifest.yaml`:

```yaml
schema_version: 1
files:
  - path: planning-duel/winner.json
    blob: files/planning-duel/winner.json
    media_type: application/json
    sha256: "<64 lowercase hex chars>"
    size_bytes: 1234
    created_by: codex:gpt-5.5
    created_at: 2026-05-11T00:00:00Z
```

Artifact paths must be relative, UTF-8, slash-separated, canonical paths and must not contain `.`, `..`, or leading `./` components. Writers that ingest hand-authored manifests should normalize leading `./` before validation. `sha256` must be a 64-character lowercase hex SHA-256 digest; writer code should format digest bytes with lowercase hex (`{:x}`), not uppercase.

The bundle format does not guarantee cross-file transactions. Writers must keep single-file updates atomic and keep partial multi-file states readable; generated repair/indexing commands reconcile cases such as appended events before envelope status rewrite or artifact files written before manifest rewrite.

## Cutover

Cutover from the current pre-reset task schema must:

1. Create or reuse `.orbit/config.yaml` with a stable `workspace_id`.
2. Allocate a canonical `ORB-00000` ID.
3. Record allocation and workspace binding metadata in `~/.orbit/tasks/index.sqlite`.
4. Materialize the canonical bundle under `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/`.
5. Create `.orbit/tasks/<task-id>` as a symlink to the canonical bundle.
6. Move YAML `description` to `description.md`.
7. Render YAML `acceptance_criteria` into `acceptance.md`.
8. Preserve existing `plan.md`.
9. Preserve existing `execution-summary.md`.
10. Convert YAML `history` to `events.jsonl`.
11. Convert YAML `comments` to `comments.jsonl`.
12. Convert YAML `review_threads` to `review-threads/`.
13. Rewrite `task.yaml` with schema version 1 and no old ID aliases.
14. Rewrite or release active task-lock reservations.
15. Record generated status, terminal-month, relation, tag, and semantic-index rebuild inputs.

Cutover must be idempotent for interrupted local runs. A partially converted task must either repair cleanly on rerun or fail with a diagnostic that names the task ID and incomplete step. The command may emit an old-ID-to-new-ID report for humans, but that report is not a persisted lookup contract.

## Agent Signature

Last revised by `codex` on 2026-05-11.
