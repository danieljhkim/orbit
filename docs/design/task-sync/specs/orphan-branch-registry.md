# Spec: Orphan Branch Registry

The orphan branch registry is the canonical store of task bundles for a team. It lives at git ref `refs/heads/orbit/tasks` on the workspace's configured remote, has no shared history with code branches, and mirrors the workspace's `.orbit/tasks/` layout exactly. Sync-enabled mutations fetch this ref before allocating IDs or writing bundles, and push afterward; atomic git ref update is the coordinator. This spec is prescriptive — it names the invariants the registry must hold, the failure modes the implementation must handle, and the migration paths between registry states. Rationale lives in [4_decisions.md](../4_decisions.md).

## Why This Exists

Without a registry, two operators on different machines have no way to see each other's tasks short of inspecting commits or PRs. The workspace task store is `WorkspaceOnly` ([scope.rs](../../../../crates/orbit-store/src/scope.rs)) and produces colliding `T<YYYYMMDD>-<N>` IDs when allocated independently. A coordinator daemon would solve both problems but contradicts the v1 per-engineer doctrine ([POSITIONING](../../../POSITIONING.md)). Git is already the team's coordination substrate; an orphan branch turns that substrate into the registry transport without adding new infrastructure.

## On-Branch Layout

The registry tree mirrors the workspace task root passed into `TaskFileStore::new` ([layout.rs:24](../../../../crates/orbit-store/src/file/task_store/layout.rs)):

```
<tree root>
├── proposed/<task-id>/{task.yaml, plan.md, execution-summary.md, artifacts/**}
├── friction/<task-id>/...
├── backlog/<task-id>/...
├── someday/<task-id>/...
├── in_progress/<task-id>/...
├── review/<task-id>/...
├── blocked/<task-id>/...
├── done/<yyyy-mm>/<task-id>/...           # date-partitioned terminal state
├── archived/<yyyy-mm>/<task-id>/...        # date-partitioned terminal state
├── rejected/<yyyy-mm>/<task-id>/...        # date-partitioned terminal state
└── _tombstones/<task-id>.yaml              # records of deleted tasks
```

The on-branch tree root maps directly to `.orbit/tasks/`. There is no translation layer. A `pull` writes branch-tree paths into the workspace at the corresponding location; a `push` writes workspace paths into the branch tree at the same location.

Files explicitly NOT on the registry:

- `.orbit/state/scoreboard/*.json` — scoreboards (out of scope, see [4_decisions.md ADR-003](../4_decisions.md))
- `.locks/**` — file locks (out of scope, per-machine ephemeral)
- `.orbit/runs/**` — job runs (out of scope, separate design)
- `.orbit/knowledge/**` — knowledge graph (out of scope, separate design)

## Commit Shape

Every push contains exactly one logical operation. Commit messages have a structured body:

```
<verb> <task-id>: <short description>

operation: <op-kind>
task-id: <task-id>
agent: <model-or-role>
host: <hostname>
parent: <parent-commit>

[T<task-id>]
```

Where:

- `<verb>` is one of: `add`, `update`, `transition`, `comment`, `archive`, `delete`.
- `<op-kind>` is one of the operation kinds enumerated below in [Operation Kinds](#operation-kinds).
- `agent` records the agent role/model that produced the operation (matches the `agent` provenance in audit events).
- `host` is the hostname for attribution; not used for routing or ACL.
- `parent` is the parent commit's short hash (redundant with git's parent edge but human-useful when reading messages).
- The bracketed `[T<task-id>]` line at the end is the standard local commit search key. It is not a graph-attribution reference and is not intended to resolve on another engineer's machine; cross-engineer references use `external_refs`.

Commits are signed with the operator's git config. The agent identity is in the structured body, not the commit author — author/committer remain the human operator's git identity.

## Operation Kinds

The fixed set of operations the sync coordinator recognizes. Each maps to a replay rule.

| Operation kind | Replay behavior on push reject |
|----------------|--------------------------------|
| `task.add` | Re-fetch, re-allocate next ID against new tip, rewrite bundle locally with new ID, retry push. |
| `task.transition` | Re-fetch; check task's current status on registry. If matches operation's expected source state, retry. If not, surface structured conflict. |
| `task.comment.append` | Re-fetch task, re-append local comment to fresh comments list, retry. Always converges. |
| `task.history.append` | Same as `comment.append`. Always converges. |
| `task.review.append` | Same as `comment.append`. Always converges. |
| `task.field.update` | Re-fetch; check field's current value matches operation's expected baseline. If matches, retry. If not, surface structured conflict. |
| `task.artifacts.upsert` | Per-artifact: if artifact path is new on registry, surface conflict; if registry version is identical, no-op; otherwise overwrite. |
| `task.delete` | Replace task path with tombstone marker. Tombstones supersede any stale offline-edit revival. |

## Invariants

The implementation must hold these invariants. A push that would violate one is rejected at the coordinator layer before reaching the remote.

1. **Path uniqueness for active tasks.** A given `<task-id>` appears under at most one status directory at any commit. If a transition would produce a state where the same task exists under two paths, the transition is invalid. This is the structural defense against the "task in two states" failure mode.
2. **Tombstone supremacy.** A tombstone for `<task-id>` invalidates any operation that would create or modify a task at `<task-id>` after the tombstone's commit. Resurrection is forbidden; reuse of the ID is forbidden.
3. **Append-only history.** The `history` field inside `task.yaml` is append-only across commits. An operation that shortens the history list is invalid.
4. **Format preservation.** Every task created via `task.add` has an ID matching `T\d{8}-\d+`. The allocator is permitted to retry across counters but never to deviate from format.
5. **Single operation per commit.** Each commit on the registry corresponds to exactly one operation. Batching is not supported in v2; multi-operation transactions, if needed, are a future-work item ([3_vision.md](../3_vision.md)).
6. **Materialized-state convergence.** For two operators with identical fetched registry tips and no pending local operations, materialized `.orbit/tasks/` content is identical. A divergence indicates a bug in `pull`.

## Failure Modes

### Network unavailable during mutation

When `enabled = true` and the network is unavailable, mutations fail with an explicit error. The error message references the `[task.sync]` config flag and offers `--offline` as the documented escape hatch. Local-only writes via `--offline` are flagged in the task's history with an `offline_write_pending_sync` marker; subsequent `task sync push` resolves them.

There is no silent fallback to local-only writes. Silent fallback is the failure mode that produces unmergeable divergent state.

### Push rejected (non-fast-forward)

The coordinator re-fetches, applies the operation's replay rule, and retries. Retry budget is `retry_max_attempts` (default 5) with exponential backoff starting at `retry_base_delay_ms` (default 100ms). Exhausting the retry budget without success surfaces as an error to the user with the structured operation context, allowing manual intervention.

### Replay surfaces a structured conflict

When an operation's baseline check fails (e.g., concurrent `task.field.update` on the same field), the coordinator writes a `.orbit/tasks/_conflicts/<task-id>.yaml` file containing both sides and aborts the push. The user runs `orbit task sync resolve <task-id>` to resolve, which produces the next push.

### Auth failure (token expired, key revoked)

Auth failures surface as the same errors `git push` would produce. Orbit does not attempt to refresh tokens or rotate credentials; that's the system credential helper's responsibility.

### Detached HEAD on registry branch

Reads against a detached HEAD are an error. Writes against a detached HEAD are an error. The coordinator never invents a synthetic branch name to paper over this — the user must explicitly check out the configured branch.

### Branch ref does not exist on remote

This is the `task sync push --init` case. The coordinator verifies the ref's absence (or emptiness) before seeding to avoid clobbering an existing registry. If the ref exists with content, `--init` refuses; the user must use a regular `pull`/`push` flow instead.

### Tombstone race

Two operators delete the same task simultaneously. Both write tombstones; the second push is rejected; replay re-fetches and sees the existing tombstone. Operation is idempotent — second deletion is a no-op, no error.

## Migration Paths

### Workspace turning sync on for the first time (no existing registry)

1. User sets `[task.sync] enabled = true` in `.orbit/config.toml`.
2. User runs `orbit task sync push --init`.
3. Coordinator fetches the configured ref. If ref does not exist on remote, proceeds. If ref exists with non-empty tree, refuses with an error directing the user to `orbit task sync pull` instead.
4. Coordinator creates an orphan commit with the workspace's `.orbit/tasks/` content.
5. Coordinator pushes. On failure, retries per [Failure Modes](#failure-modes).
6. From this point, mutations are online-only.

### Workspace turning sync on for an existing registry

1. User sets `[task.sync] enabled = true`.
2. User runs `orbit task sync pull`.
3. Coordinator fetches the registry. Detects local-only tasks (present in `.orbit/tasks/` but absent from registry).
4. For each local-only task, the coordinator surfaces a choice: publish (push to registry) or stash (move to `.orbit/tasks/_local/<task-id>/` for later). Default is publish; `--stash-local` flips the default.
5. Coordinator overlays registry content onto the workspace tree, preserving local-only changes per the choice above.
6. From this point, mutations are online-only.

### Onboarding a new engineer

1. New engineer clones the repo and installs Orbit.
2. New engineer copies `.orbit/config.toml` from a teammate (or the team's standard config) with `[task.sync] enabled = true`.
3. New engineer runs `orbit workspace init && orbit task sync pull`.
4. Coordinator fetches the registry and materializes `.orbit/tasks/` from it.
5. From this point, mutations are online-only.

### Disabling sync on a previously-synced workspace

1. User sets `[task.sync] enabled = false`.
2. Workspace tasks remain on disk; subsequent mutations are local-only.
3. The remote registry is unchanged. If the user later re-enables sync, they re-enter the "existing registry" migration path above.
4. There is no "delete the remote registry" command at the workspace level. That's a manual `git push --delete` or branch-removal operation on the remote, intentionally not exposed by Orbit.

## Agent Signature

Authored by `claude` (claude-opus-4-7) under [T20260505-12]. Codex (`gpt-5.5`) won the planning duel for this task and shaped the call-site enumeration and config schema.
