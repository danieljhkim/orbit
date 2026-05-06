# Task Sync — Design

**Status:** Draft
**Owner:** claude
**Last updated:** 2026-05-06

This document specifies the v2 design for task sync: the registry mechanism, the conflict-resolution model and why it's the central design question, the call sites that need to become sync-aware, the CLI surface, the config schema, and the migration paths. v1 ships with sync disabled by default and the code path absent. The architectural boundary is explicit: the task store ([orbit-store/src/file/task_store/](../../../crates/orbit-store/src/file/task_store/)) keeps owning YAML layout and validation; a new task-sync coordinator above the store owns git transport.

---

## 1. Scope

In scope for task sync:

- Task YAML bundles (`task.yaml`).
- Companion files: `plan.md`, `execution-summary.md`.
- Task artifacts under `<task-id>/artifacts/**`.
- ID allocation against the registry's view of state, with the format `T<YYYYMMDD>-<N>` preserved for the future sync registry. Current v1 Orbit `task_id` values remain local-only search keys after [T20260506-11]; cross-engineer references use `external_refs`.

Out of scope (with rationale in [§7](#7-concerns--honest-limitations)):

- File locks, audit DB, scoreboards, job runs, knowledge DB, SQLite task reservations.

---

## 2. Mechanism

### 2.1 Registry ref and branch name

The registry lives at `refs/heads/orbit/tasks`. User-facing branch name: `orbit/tasks`. This is a normal-form git branch ref so:

- Branch protection on hosts (GitHub, GitLab, Gitea) can be applied without custom configuration.
- Code review tooling that lists branches will include it.
- `git log orbit/tasks` and `git diff main..orbit/tasks` work without ceremony.
- Users who don't care about it never have to look at it; users who do care can inspect it with the same tools they use for code branches.

The alternative (`refs/orbit/tasks` — outside `refs/heads/`) was considered and rejected. It hides the ref from `git branch -a` (a small UX win) but breaks every host-side branch-protection feature, requires custom fetchspecs, and fights every code-review tool. The tradeoff is documented in [4_decisions.md ADR-001](./4_decisions.md).

### 2.2 Orphan history

The branch is created with `git checkout --orphan` (or equivalent in `git2`). It has no shared commits with `main`. Every commit on the branch represents a task lifecycle event. Merging the registry into product branches is meaningless and prevented by host-side branch protection in deployments that use it.

### 2.3 On-branch layout

The branch tree mirrors `.orbit/tasks/` exactly:

```
refs/heads/orbit/tasks (tree root)
├── proposed/
│   └── T20260505-12/
│       ├── task.yaml
│       ├── plan.md
│       ├── execution-summary.md
│       └── artifacts/                (optional)
├── backlog/
│   └── T20260504-7/
│       ├── task.yaml
│       └── ...
├── in_progress/
│   └── T20260503-2/
│       └── ...
├── done/
│   └── 2026-05/                       (date-partitioned for terminal states)
│       └── T20260501-3/
│           └── ...
├── archived/
│   └── 2026-05/
│       └── ...
└── rejected/
    └── 2026-04/
        └── ...
```

This layout matches the existing workspace task root passed into `TaskFileStore::new` (see [layout.rs:24](../../../crates/orbit-store/src/file/task_store/layout.rs)). Reusing the layout means the registry tree can be hydrated into `.orbit/tasks/` directly without translation, and `git log -- proposed/T20260505-12/task.yaml` works for inspecting a task's history.

### 2.4 Commit shape

Each push contains exactly one logical operation, with a structured message:

```
<verb> <task-id>: <short description>

operation: <op-kind>
task-id: <task-id>
agent: <model-or-role>
host: <hostname>
parent: <parent-commit>

[T20260505-12]
```

Where `<verb>` is one of `add`, `update`, `transition`, `comment`, `archive`, `delete`, and `<op-kind>` is the structured operation name (see [§3.2](#32-operation-aware-replay-the-recommended-mechanism)). The `agent` and `host` lines satisfy the auditability non-negotiable: every change to a task is attributable to a concrete agent identity on a concrete machine.

Commits are signed with the operator's git config (same identity as code commits). Auth piggybacks on whatever credential helper the operator uses for `git push origin main` — see [4_decisions.md ADR-005](./4_decisions.md).

---

## 3. Conflict Resolution

This is the central design question. Standard git text merge cannot handle task-bundle structure honestly. Four options were evaluated against four concrete scenarios:

- **(a) Counter collision on concurrent ADD.** Engineer A and engineer B each call `task add` on 2026-05-05 within the same second, before either has fetched the other's commit. Both compute `T20260505-13` from local state.
- **(b) Status transition divergence.** Engineer A moves [T20260504-7] from `backlog/` to `in_progress/`. Engineer B simultaneously moves the same task from `backlog/` to `archived/2026-05/`. Both push.
- **(c) Concurrent comment append.** Engineer A and engineer B each append a different comment to the `comments: []` array in [T20260504-7]/task.yaml. Both push.
- **(d) Concurrent same-field edit.** Engineer A edits the `description` of [T20260504-7]. Engineer B edits the same `description` differently. Both push.

### 3.1 The four options compared

| Option | (a) ADD collision | (b) Status divergence | (c) Comment append | (d) Field edit |
|--------|-------------------|------------------------|--------------------|-----------------|
| **1. ADD-only sync** | Auto: re-fetch, re-allocate `-14`, push. | Stays local; not synced. | Stays local; not synced. | Stays local; not synced. |
| **2. Operation-aware replay (recommended)** | Auto: re-fetch, re-allocate, push. | Re-fetch, see remote moved task to `archived/`; if local op was a transition from `backlog/`, refuse and surface to user — source state is no longer valid. | Auto: re-fetch task, re-append local comment to the new comments list, push. | Re-fetch, detect concurrent edit to the same field, surface to user as a structured conflict. |
| **3. Event-sourced** | Auto: events have monotonic IDs; rebase appends. | Auto: both events serialize on rebase; final state is the result of applying both in commit order. Last operator wins. | Auto: both append events; both visible in materialized state. | Auto: last event in commit order wins; first edit visible in event log. |
| **4. No sync** | N/A — no sync. | N/A. | N/A. | N/A. |

### 3.2 Operation-aware replay: the recommended mechanism

The chosen mechanism for v2. Key claim: the orphan branch holds canonical YAML *snapshots*, but the client treats every push as an *operation* and replays operations on push reject. The operation kinds are:

| Operation | Replay behavior |
|-----------|-----------------|
| `task.add` | Re-fetch, re-allocate next ID against new tip, rewrite bundle locally with new ID, retry push. |
| `task.transition` | Re-fetch, check that task's current status on the registry still matches the operation's expected source state. If yes, retry. If no (someone else moved the task), surface conflict — user must inspect and re-issue. |
| `task.comment.append` | Re-fetch task YAML, append the new comment to the fresh comments list, push. Always converges. |
| `task.history.append` | Same as comments — append-only by construction. Always converges. |
| `task.review.append` | Same. |
| `task.field.update` (description, priority, plan, etc.) | Re-fetch, compute diff between operation's expected baseline and registry's current value. If unchanged on registry, retry. If changed, surface structured conflict. |
| `task.external_refs.merge` | Re-fetch task YAML and union `external_refs` by `(system, id)`. Distinct keys from both replicas are preserved. When both replicas write the same `(system, id)` key with different `url` values, keep one entry and let the later replayed operation win for `url` only. The rest of the task remains governed by the operation that caused the replay. |
| `task.artifacts.upsert` | Per-artifact: if the artifact is new on the registry too, surface conflict; if it's the same, no-op; otherwise overwrite. |
| `task.delete` | Always replaces the task with a tombstone marker (see [§3.4](#34-deletion-via-tombstone)). |

The "structured conflict" path is critical. When operations cannot replay automatically, the client writes a `.orbit/tasks/_conflicts/<task-id>.yaml` describing both sides and exits with a clear message. The user runs `orbit task sync resolve <task-id>` to choose, edit, or merge, and the resolution becomes the next push.

`external_refs` was added as pure task metadata in [T20260506-9]. This document records the intended task-sync merge behavior for any future task-sync coordinator work, but the current sync layer does not yet expose implemented field-level replay hooks. The previously tracked standalone replay follow-up [T20260506-13] was rejected as no longer required.

### 3.3 Why not the other three

**Option 1 (ADD-only) was rejected** because the operations that update most often — comments, history, status transitions — are exactly the ones that wouldn't sync. Engineer B sees engineer A's task created, but never sees A pick it up. The mental model "I see your tasks but not your progress" is harder to internalize than "I see nothing." Half-coverage is worse than no coverage when the partial coverage is the rarely-changing part.

**Option 3 (event-sourced) was rejected for v2** because the architectural shift is not justified by the conflict-resolution gain. Operation-aware replay handles the same scenarios with materially less churn: the on-branch artifact remains a YAML snapshot inspectable with standard git tooling, the existing task store keeps owning YAML, and no event-replay materializer needs to be built. Event sourcing remains a candidate for a future iteration if the operation-replay model proves insufficient — see [3_vision.md §1](./3_vision.md).

**Option 4 (no sync) was rejected** because the v1 per-engineer doctrine is honest about *what doesn't sync*, but it doesn't justify never building any sync. Once v2 ships shared-host, sync of some kind is required; an opt-in git-based sync is a less-disruptive incremental step than a coordinator daemon and serves teams that want visibility without infrastructure.

### 3.4 Deletion via tombstone

`task.delete` does *not* remove the task directory from the orphan branch. It writes a `_tombstones/<task-id>.yaml` entry recording the deletion timestamp and the deleting agent, and the regular task path is removed. Reads ignore tombstoned IDs. This preserves the auditability invariant — the history of a deleted task is still inspectable via `git log refs/heads/orbit/tasks -- 'proposed/<task-id>/'` — and prevents the "I deleted my task and it came back when engineer B pushed an old version" footgun.

---

## 4. Call Sites

Every sync-aware mutation must route through the new task-sync coordinator (introduced in [§5.1](#51-architectural-boundary)). The call sites that need changes:

### 4.1 In `crates/orbit-store/src/file/task_store/`

| File:line | Function | Change |
|-----------|----------|--------|
| [api.rs:28](../../../crates/orbit-store/src/file/task_store/api.rs) | `create_task` | Wrap with sync-coordinator call: fetch registry, then allocate, then write, then commit + push. |
| [layout.rs:102-132](../../../crates/orbit-store/src/file/task_store/layout.rs) | `next_task_id` | Allocator must accept a "view" abstraction so it can scan against fetched registry state plus local unpushed tasks, while preserving the `T<YYYYMMDD>-<N>` format exactly. |
| [api.rs:174](../../../crates/orbit-store/src/file/task_store/api.rs) | `update_task_document` | Wrap: fetch, detect remote movement, apply replay rules from [§3.2](#32-operation-aware-replay-the-recommended-mechanism), push. |
| [api.rs:281](../../../crates/orbit-store/src/file/task_store/api.rs) | `update_task_history` | Same; uses `task.history.append` operation kind (always-converging). |
| [api.rs:342](../../../crates/orbit-store/src/file/task_store/api.rs) | `update_task_reviews` | Same; uses `task.review.append`. |
| [api.rs:374](../../../crates/orbit-store/src/file/task_store/api.rs) | `upsert_task_artifacts` | Per-artifact replay; see [§3.2](#32-operation-aware-replay-the-recommended-mechanism). |
| [api.rs:389](../../../crates/orbit-store/src/file/task_store/api.rs) | `persist_bundle_update` | Central local mutation point. Status-directory move is the most conflict-prone operation; replay rule from [§3.2](#32-operation-aware-replay-the-recommended-mechanism) applies. |
| [api.rs:411](../../../crates/orbit-store/src/file/task_store/api.rs) | `delete_task` | Tombstone semantics per [§3.4](#34-deletion-via-tombstone). |
| [bundle.rs:26](../../../crates/orbit-store/src/file/task_store/bundle.rs) | `write_bundle_for_state` | Materializes `task.yaml` + `plan.md` + `execution-summary.md` to disk. The sync coordinator reads from this output and stages it into the registry tree. No direct git transport in this function. |
| [bundle.rs:34](../../../crates/orbit-store/src/file/task_store/bundle.rs) | `write_bundle_at` | Same. |
| [layout.rs:176](../../../crates/orbit-store/src/file/task_store/layout.rs), [:180](../../../crates/orbit-store/src/file/task_store/layout.rs), [:201](../../../crates/orbit-store/src/file/task_store/layout.rs), [:326](../../../crates/orbit-store/src/file/task_store/layout.rs) | State, task, artifact path helpers and status-move semantics | Layout helpers are reused as-is; the sync coordinator constructs equivalent on-branch paths via the same helpers. |

### 4.2 Upstream entry points

| File:line | Role |
|-----------|------|
| [crates/orbit-core/src/command/task/add.rs:23](../../../crates/orbit-core/src/command/task/add.rs) | Runtime task-add entry point. |
| [crates/orbit-core/src/command/task/update.rs:58](../../../crates/orbit-core/src/command/task/update.rs) | Runtime task-update entry point. |
| [crates/orbit-cli/src/command/task/add.rs:69](../../../crates/orbit-cli/src/command/task/add.rs) | CLI `orbit task add`. |
| [crates/orbit-cli/src/command/task/update.rs:72](../../../crates/orbit-cli/src/command/task/update.rs) | CLI `orbit task update`. |
| [crates/orbit-cli/src/command/web/api.rs:357](../../../crates/orbit-cli/src/command/web/api.rs) | Web dashboard task-create handler. |
| [crates/orbit-cli/src/command/web/api.rs:387](../../../crates/orbit-cli/src/command/web/api.rs) | Web dashboard task-update handler. |
| [crates/orbit-cli/src/command/task/command.rs:34](../../../crates/orbit-cli/src/command/task/command.rs) | Where the new `Sync(TaskSyncCommand)` subcommand attaches under `TaskSubcommand`. |

All four mutation entry points (runtime, CLI, web) route through the same sync-aware service so the policy is consistent regardless of how a mutation arrives.

---

## 5. Architectural Boundary

### 5.1 The sync coordinator

A new component, tentatively `orbit-store::sync::TaskSyncCoordinator`, sits *above* the existing file store and *below* the runtime/CLI/web entry points. It owns:

- Git transport (fetch, commit, push, retry).
- Operation classification (which task operation is being attempted).
- Replay logic (the rules in [§3.2](#32-operation-aware-replay-the-recommended-mechanism)).
- Auth handoff to the system git credential helper.

It does *not* own:

- YAML serialization (still in [bundle.rs](../../../crates/orbit-store/src/file/task_store/bundle.rs)).
- Layout (`<state>/<task-id>/...` paths still in [layout.rs](../../../crates/orbit-store/src/file/task_store/layout.rs)).
- Reservation locks (still per-machine in [task_reservation_store.rs](../../../crates/orbit-store/src/sqlite/task_reservation_store.rs); locks are out of scope at any version).

Putting git transport above the file store keeps `orbit-store` mockable for tests and ensures sync policy can change without rewriting layout code.

### 5.2 git2 vs shelling to git

The recommendation is the `git2` crate (libgit2 bindings). Reasons:

- In-process control over fetch/push/commit lets the coordinator implement retry without spawning subprocesses.
- Authentication callbacks integrate with the system credential helper without parsing `git` output.
- Errors are typed; subprocess error parsing is brittle.

The cost is acknowledged: `git2` has a steeper learning curve, occasional ABI churn, and a larger binary footprint. See [4_decisions.md ADR-006](./4_decisions.md).

---

## 6. Config and CLI

### 6.1 Config schema

Workspace-level configuration in `.orbit/config.toml`:

```toml
[task.sync]
enabled = false                              # default-off; preserves v1 per-machine behavior
remote = "origin"                            # the git remote to fetch/push against
ref = "refs/heads/orbit/tasks"               # the registry ref
fetch_before_read = false                    # if true, every read does a background fetch first
retry_max_attempts = 5                       # exponential backoff on non-fast-forward
retry_base_delay_ms = 100
```

`enabled = false` (or absent `[task.sync]`) is the v1 and v2-default behavior: workspaces continue exactly as today with no network calls, no orphan branch, no sync coordinator on the mutation path.

`enabled = true` activates online-mode for `task add` and mutating `task update`. Read paths (`task list`, `task show`) remain offline by default; `fetch_before_read = true` is an opt-in for teams that prefer freshness over latency.

### 6.2 CLI surface

Three new subcommands under `orbit task sync`:

| Command | Behavior |
|---------|----------|
| `orbit task sync status` | Compares local task tree to fetched registry. Lists tasks that are local-only, registry-only, or have diverged content. Read-only. |
| `orbit task sync pull` | Fetches the registry ref and updates `.orbit/tasks/` from it. Local-only changes are surfaced; user must explicitly stash or push them. |
| `orbit task sync push` | Pushes local task changes. Used for the rare cases where automatic per-mutation push has been bypassed (e.g., `--offline-add` flag during a network outage). |
| `orbit task sync push --init` | First-time enablement on a workspace with existing tasks. Verifies the registry ref does not exist (or is empty), then seeds it from local `.orbit/tasks/`. |
| `orbit task sync resolve <task-id>` | Walks the user through a structured conflict written by the replay logic. Conflicts live at `.orbit/tasks/_conflicts/<task-id>.yaml` until resolved. |

### 6.3 Failure modes

When `enabled = true` and the network is unavailable, mutations fail with an explicit error referencing the config flag. There is no silent fallback to local-only writes — silent fallback is exactly the failure mode that produces divergent state. A user who knows they want offline behavior temporarily can pass `--offline` to allow local-only writes; those writes are flagged in the local task history and require an explicit `task sync push` to publish, with conflict resolution if applicable.

---

## 7. Concerns & Honest Limitations

### 7.1 Out of scope (at any version)

The following are explicitly NOT synced by task sync, regardless of v1/v2 status:

- **File locks ([task_reservation_store.rs](../../../crates/orbit-store/src/sqlite/task_reservation_store.rs)).** Locks are ephemeral, TTL-based, and per-machine by design. Cross-machine lock coordination is a different problem with different consistency requirements; it belongs to v2 shared-host work, not task sync.
- **Audit DB (`~/.orbit/orbit.db`).** The audit store is `GlobalOnly` per the [scoping rules](../../../crates/orbit-store/src/scope.rs). Cross-machine aggregation of audit events is its own design problem and has different retention, query, and tamper-evidence requirements than task bundles.
- **Scoreboards (`.orbit/state/scoreboard/*.json`).** Scoreboards use read-modify-write counter semantics that don't merge cleanly. Reconciliation requires either a coordinator or a fundamentally different model (event-sourced counters), neither of which fits inside task sync.
- **Job runs (`.orbit/runs/`).** Job-run YAML bundles can be large, contain blob refs, and have their own lifecycle. They are workspace-local execution artifacts; making them team-shared is a separate design.
- **Knowledge graph (`.orbit/knowledge/`).** The graph is content-addressed and branch-scoped; sharing it has its own design under [docs/design/knowledge-graph/](../knowledge-graph/).

### 7.2 Online-only mutations

Sync-enabled workspaces lose offline `task add` and `task update`. This is deliberate — making git's atomic ref update the coordinator requires the network at mutation time — but it's a real cost. Users who need genuinely offline task creation should either keep sync disabled or accept the `--offline` escape hatch with its conflict-resolution overhead. A team that runs `task add` from CI or air-gapped environments would need a different approach.

### 7.3 Last-writer-wins for the replay-could-not-resolve case

Even with operation-aware replay, some scenarios genuinely cannot resolve without human input — e.g., two engineers editing the same `description` field. The design surfaces these as structured conflicts rather than silently last-writer-wins, but the cost is that the user has to think about a merge they didn't expect. The `task sync resolve` UX needs to be excellent for this not to be friction.

### 7.4 Orphan-branch growth

Every status transition is a commit. A team with high task throughput will accumulate commits on `refs/heads/orbit/tasks` indefinitely. v2 ships without compaction; once the branch becomes operationally painful, a follow-up design adds snapshot-and-prune semantics. See [3_vision.md §1](./3_vision.md).

### 7.5 Soft-claim is advisory, not locking

The design assumes a future `assigned_to` field can act as a soft-claim hint ("Bob said he's working on this"), but it does *not* prevent two engineers from working the same task simultaneously. Real cross-machine locking is out of scope; soft-claim is a coordination convention, not a guarantee.

### 7.6 Branch protection and host-side tooling

The orphan branch convention assumes the team's git host supports branch protection (or that the team is comfortable with unprotected refs). Hosts that don't support this — or self-hosted setups without it configured — risk accidental merge of `orbit/tasks` into product branches. The design assumes the team will set branch protection but does not enforce it at the Orbit layer.

### 7.7 Auth surface inheritance

Task sync inherits whatever auth posture the team uses for `git push`. If a team uses SSO-wrapped tokens that expire mid-day, `task add` will fail mid-flow at refresh time. Orbit does not handle this: the user gets the same error they would get from `git push`, which is appropriate for a tool that's supposed to be a thin layer over git.

---

## Task References

- [T20260505-12] — Design git-orphan-branch task sync (v2 feature). The task that produced this folder.
- [T20260421-0528] — Historical knowledge-graph task attribution. Superseded as a canonical load-bearing ID example by [T20260506-11].
- [T20260506-9] — Adds first-class task `external_refs` metadata and documents the task-sync merge rule.
- [T20260506-13] — Rejected follow-up; no standalone task-sync replay implementation is currently required for `external_refs`.

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
